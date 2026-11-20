//! A StatusNotifierItem backed by the existing Tokio/zbus runtime.
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const ITEM: &str = "/StatusNotifierItem";
const MENU: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const POLL: Duration = Duration::from_secs(1);
const LEASE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
pub(super) enum Action {
    Show,
    Hide,
    Quit,
}

pub(super) struct Tray {
    verified: Arc<Mutex<Option<Instant>>>,
    actions: mpsc::Receiver<Action>,
    _stop: futures_channel::oneshot::Sender<()>,
}

impl Tray {
    pub(super) fn start() -> Self {
        let (send, actions) = mpsc::sync_channel(16);
        let verified = Arc::new(Mutex::new(None));
        // spawn_core starts eagerly; nothing on the GPUI thread waits on D-Bus.
        let (stop, stopped) = futures_channel::oneshot::channel();
        let lease = verified.clone();
        drop(crate::runtime::spawn_core(async move {
            tokio::select! {
                _ = stopped => {},
                _ = watch(send, lease) => {},
            }
        }));
        Self {
            verified,
            actions,
            _stop: stop,
        }
    }

    pub(super) fn available(&self) -> bool {
        fresh(*self.verified.lock().expect("tray lease"), Instant::now())
    }

    pub(super) fn actions(&self) -> impl Iterator<Item = Action> + '_ {
        self.actions.try_iter().take(16)
    }
}

fn fresh(verified: Option<Instant>, now: Instant) -> bool {
    verified.is_some_and(|verified| now.saturating_duration_since(verified) < LEASE)
}

async fn watch(send: mpsc::SyncSender<Action>, verified: Arc<Mutex<Option<Instant>>>) {
    loop {
        if let Ok(Ok(connection)) = tokio::time::timeout(POLL, async {
            zbus::connection::Builder::session()?
                .serve_at(ITEM, Item(send.clone()))?
                .serve_at(MENU, Menu(send.clone()))?
                .build()
                .await
        })
        .await
        {
            loop {
                let result = tokio::time::timeout(POLL, verify(&connection)).await;
                *verified.lock().expect("tray lease") =
                    matches!(result, Ok(Ok(true))).then(Instant::now);
                if !matches!(result, Ok(Ok(_))) {
                    break;
                }
                tokio::time::sleep(POLL).await;
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn verify(connection: &zbus::Connection) -> zbus::Result<bool> {
    let watcher: zbus::Proxy<'_> = zbus::proxy::Builder::new(connection)
        .destination(WATCHER)?
        .path("/StatusNotifierWatcher")?
        .interface(WATCHER)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    if !watcher
        .get_property::<bool>("IsStatusNotifierHostRegistered")
        .await?
    {
        return Ok(false);
    }
    let name = connection.unique_name().expect("session bus unique name");
    let item = format!("{name}{ITEM}");
    let registered = |items: Vec<String>| items.iter().any(|entry| entry == &item);
    if registered(
        watcher
            .get_property("RegisteredStatusNotifierItems")
            .await?,
    ) {
        return Ok(true);
    }
    // A path registers the caller's unique bus name, including when running multiple instances.
    watcher
        .call::<_, _, ()>("RegisterStatusNotifierItem", &(ITEM,))
        .await?;
    Ok(registered(
        watcher
            .get_property("RegisteredStatusNotifierItems")
            .await?,
    ))
}

struct Item(mpsc::SyncSender<Action>);

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.0.try_send(Action::Show);
    }

    #[zbus(property)]
    fn category(&self) -> &str {
        "Communications"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        super::APP_ID
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Stonks Chat"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str_unchecked(MENU)
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        let image =
            image::load_from_memory(include_bytes!("../../assets/branding/32x32.png"))
                .expect("bundled tray icon")
                .into_rgba8();
        let (width, height) = image.dimensions();
        let mut bytes = image.into_raw();
        for rgba in bytes.chunks_exact_mut(4) {
            rgba.rotate_right(1); // SNI uses ARGB32 in network byte order.
        }
        vec![(width as i32, height as i32, bytes)]
    }
}

struct Menu(mpsc::SyncSender<Action>);
type Properties = HashMap<String, OwnedValue>;
type Layout = (i32, Properties, Vec<OwnedValue>);
const ENTRIES: [(i32, &str, Action); 3] = [
    (1, "Show Stonks Chat", Action::Show),
    (2, "Hide Stonks Chat", Action::Hide),
    (3, "Quit", Action::Quit),
];

fn properties(id: i32, names: &[String]) -> Properties {
    ENTRIES
        .iter()
        .find(|entry| entry.0 == id)
        .filter(|_| names.is_empty() || names.iter().any(|name| name == "label"))
        .map(|entry| HashMap::from([("label".into(), zbus::zvariant::Str::from(entry.1).into())]))
        .unwrap_or_default()
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(out_args("revision", "layout"))]
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> (u32, Layout) {
        let children = if parent_id == 0 && recursion_depth != 0 {
            ENTRIES
                .iter()
                .map(|&(id, _, _)| {
                    OwnedValue::try_from(Value::from((
                        id,
                        properties(id, &property_names),
                        Vec::<OwnedValue>::new(),
                    )))
                    .expect("static tray menu layout")
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut props = properties(parent_id, &property_names);
        if parent_id == 0
            && (property_names.is_empty()
                || property_names.iter().any(|name| name == "children-display"))
        {
            props.insert(
                "children-display".into(),
                zbus::zvariant::Str::from("submenu").into(),
            );
        }
        (1, (parent_id, props, children))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, Properties)> {
        ENTRIES
            .iter()
            .filter(|entry| ids.is_empty() || ids.contains(&entry.0))
            .map(|entry| (entry.0, properties(entry.0, &property_names)))
            .collect()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id == "clicked" {
            if let Some((_, _, action)) = ENTRIES.iter().find(|entry| entry.0 == id) {
                let _ = self.0.try_send(*action);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_requires_current_host_confirmation() {
        let now = Instant::now();
        assert!(!fresh(None, now));
        assert!(fresh(Some(now), now + POLL));
        assert!(!fresh(Some(now), now + LEASE));
    }
}

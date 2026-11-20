use super::{
    host::MusicCommand,
    protocol::MusicPacket,
    resolver::resolve_media,
    runtime::{resolver_generation_is_current, MusicRuntime},
    transport::publish_packet,
    *,
};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) async fn spawn_media_preparation(
    room: Arc<Room>,
    runtime: Arc<Mutex<MusicRuntime>>,
    item_id: String,
    source: String,
    local_id: String,
) {
    let (resolver_generation, host_generation) = {
        let mut locked = runtime.lock().await;
        locked.next_resolver_generation = locked.next_resolver_generation.saturating_add(1);
        let resolver_generation = locked.next_resolver_generation;
        locked
            .resolver_generations
            .insert(item_id.clone(), resolver_generation);
        if let Some(previous) = locked.resolver_tasks.remove(&item_id) {
            previous.abort();
        }
        (resolver_generation, locked.generation)
    };
    let task_runtime = runtime.clone();
    let task_item_id = item_id.clone();
    let task = tokio::spawn(async move {
        let Ok(media) = resolve_media(&source, false).await else {
            return;
        };
        let (state, commands) = {
            let mut locked = task_runtime.lock().await;
            if !resolver_generation_is_current(
                &locked,
                &task_item_id,
                resolver_generation,
                host_generation,
            ) {
                return;
            }
            if let Some(owned) = locked.owned_items.get_mut(&task_item_id) {
                owned.title = media.title.clone();
                owned.source_url = media.source_url.clone();
                owned.artwork_url = media.artwork_url.clone();
            }
            locked
                .prepared_media
                .insert(task_item_id.clone(), media.clone());
            locked.resolver_generations.remove(&task_item_id);
            locked.resolver_tasks.remove(&task_item_id);
            (locked.snapshot.clone(), locked.commands.clone())
        };
        let command = MusicCommand::UpdateMetadata {
            id: task_item_id.clone(),
            requester: local_id.clone(),
            title: media.title.clone(),
            source_url: media.source_url.clone(),
            artwork_url: media.artwork_url.clone(),
        };
        if state.host_id.as_deref() == Some(&local_id) {
            if let Some(commands) = commands {
                let _ = commands.send(command);
            }
        } else if state.active {
            if task_runtime.lock().await.generation != host_generation {
                return;
            }
            let _ = publish_packet(
                &room,
                &MusicPacket::Metadata {
                    session_id: state.session_id,
                    item_id: task_item_id,
                    requester: local_id,
                    title: media.title,
                    source_url: media.source_url,
                    artwork_url: media.artwork_url,
                },
            )
            .await;
        }
    });
    let mut locked = runtime.lock().await;
    if locked.generation == host_generation
        && locked.resolver_generations.get(&item_id) == Some(&resolver_generation)
    {
        locked.resolver_tasks.insert(item_id, task);
    } else {
        task.abort();
    }
}

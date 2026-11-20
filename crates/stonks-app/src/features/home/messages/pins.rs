use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
    sync::Mutex,
};

pub(super) type Scope = (String, String);
pub(super) type Pins = BTreeSet<String>;
type Store = BTreeMap<String, BTreeMap<String, Pins>>;

fn load(path: &Path) -> io::Result<Store> {
    match std::fs::File::open(path) {
        Ok(file) => Ok(serde_json::from_reader(io::BufReader::new(file))?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Store::new()),
        Err(error) => Err(error),
    }
}

pub(super) fn read(path: &Path, scope: &Scope) -> io::Result<Pins> {
    Ok(load(path)?
        .get_mut(&scope.0)
        .and_then(|accounts| accounts.remove(&scope.1))
        .unwrap_or_default())
}

pub(super) fn toggle(path: &Path, scope: &Scope, id: String) -> io::Result<Pins> {
    static WRITE: Mutex<()> = Mutex::new(());
    let _guard = WRITE
        .lock()
        .map_err(|_| io::Error::other("Pins store lock poisoned"))?;
    let mut store = load(path)?;
    let pins = store
        .entry(scope.0.clone())
        .or_default()
        .entry(scope.1.clone())
        .or_default();
    if !pins.remove(&id) {
        pins.insert(id);
    }
    let result = pins.clone();
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("No settings directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(file.as_file_mut(), &store)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|err| err.error)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_survive_reload_and_are_isolated_by_account_and_deployment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conversation-pins.json");
        let scope = |deployment: &str, account: &str| (deployment.into(), account.into());
        let first = scope("one", "alice");
        toggle(&path, &first, "dm".into()).unwrap();
        assert!(read(&path, &first).unwrap().contains("dm"));
        assert!(read(&path, &scope("one", "bob")).unwrap().is_empty());
        assert!(read(&path, &scope("two", "alice")).unwrap().is_empty());
        toggle(&path, &scope("one", "bob"), "group".into()).unwrap();
        toggle(&path, &first, "dm".into()).unwrap();
        assert!(read(&path, &first).unwrap().is_empty());
        assert!(read(&path, &scope("one", "bob")).unwrap().contains("group"));
    }

    #[test]
    fn corrupt_store_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conversation-pins.json");
        std::fs::write(&path, "invalid").unwrap();
        assert!(toggle(&path, &("one".into(), "alice".into()), "dm".into()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "invalid");
    }
}

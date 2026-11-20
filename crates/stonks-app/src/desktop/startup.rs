use super::{atomic_write, config_home, APP_ID};
use std::{env, fs, io, path::Path};

fn entry_path() -> io::Result<std::path::PathBuf> {
    Ok(config_home()?
        .join("autostart")
        .join(format!("{APP_ID}.desktop")))
}

pub(crate) fn starts_with_computer() -> io::Result<bool> {
    entry_path()?.try_exists()
}

/// The GPUI desktop entry is the only persisted state: don't also change Tauri's preference.
pub(crate) fn set_start_with_computer(enabled: bool) -> io::Result<()> {
    let entry = entry_path()?;
    update_entry(
        &entry,
        enabled.then(env::current_exe).transpose()?.as_deref(),
    )
}

fn update_entry(entry: &Path, executable: Option<&Path>) -> io::Result<()> {
    let Some(executable) = executable else {
        return match fs::remove_file(entry) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    };
    atomic_write(entry, entry_contents(executable)?.as_bytes())
}

fn entry_contents(executable: &Path) -> io::Result<String> {
    let executable = executable
        .to_str()
        .filter(|path| !path.chars().any(char::is_control) && executable.is_absolute())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "unsupported executable path")
        })?;
    let mut exec = String::new();
    for ch in executable.chars() {
        // Exec quoting is interpreted after desktop-entry string escaping.
        match ch {
            '\\' => exec.push_str(r"\\\\"),
            '"' | '$' | '`' => {
                exec.push_str(r"\\");
                exec.push(ch);
            }
            '%' => exec.push_str("%%"),
            _ => exec.push(ch),
        }
    }
    Ok(format!("[Desktop Entry]\nType=Application\nName=Stonks Chat\nExec=\"{exec}\"\nIcon={APP_ID}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_entry_is_the_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("autostart/gpui.desktop");
        update_entry(&entry, Some(Path::new("/opt/stonks"))).unwrap();
        let prior = fs::read(&entry).unwrap();
        assert!(update_entry(&entry, Some(Path::new("relative"))).is_err());
        assert_eq!(fs::read(&entry).unwrap(), prior);
        update_entry(&entry, None).unwrap();
        update_entry(&entry, None).unwrap();
        assert!(!entry.exists());
    }

    #[test]
    fn quotes_exec_without_interpreting_field_codes_or_newlines() {
        let entry = entry_contents(Path::new(r#"/opt/a b\c"$`%u"#)).unwrap();
        assert!(entry.contains(r#"Exec="/opt/a b\\\\c\\"\\$\\`%%u""#));
        assert!(entry_contents(Path::new("/opt/a\nb")).is_err());
    }
}

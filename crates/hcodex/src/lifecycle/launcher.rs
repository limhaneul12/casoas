//! Retire only the known project-owned Python command shim, never user launchers.

use std::path::Path;

#[cfg(unix)]
pub(super) fn migrate(repository: &Path, home: &Path) -> crate::Result<bool> {
    use std::fs;
    use std::io::ErrorKind;
    use std::os::unix::fs::symlink;

    let launcher = home.join(".local/bin/hcodex");
    let metadata = match fs::symlink_metadata(&launcher) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() > 4096 {
        return Ok(false);
    }
    let previous = fs::read(&launcher)?;
    let known_launcher = ["python", "python3"].iter().any(|interpreter| {
        let expected = format!(
            "#!/bin/sh\nexport PYTHONPATH=\"{}\"\nexec \"{}/.venv/bin/{interpreter}\" -m app.core.ctl \"$@\"\n",
            repository.display(),
            repository.display(),
        );
        previous == expected.as_bytes()
    });
    if !known_launcher {
        return Ok(false);
    }
    let binary = super::config::resolve_hcodex_binary(repository)?;
    let staged = launcher.with_file_name(format!(".hcodex-{}.tmp", uuid::Uuid::new_v4().simple()));
    symlink(binary, &staged)?;
    // Recheck after staging so an intervening ordinary user edit is preserved.
    let replaced = (|| -> crate::Result<bool> {
        if !fs::symlink_metadata(&launcher)?.is_file() || fs::read(&launcher)? != previous {
            return Ok(false);
        }
        fs::rename(&staged, &launcher)?;
        Ok(true)
    })();
    if !matches!(replaced, Ok(true)) {
        fs::remove_file(staged)?;
    }
    replaced
}

#[cfg(not(unix))]
pub(super) fn migrate(_repository: &Path, _home: &Path) -> crate::Result<bool> {
    Ok(false)
}

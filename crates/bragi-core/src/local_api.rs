//! What Bragi's local APIs share: where the Bragi home is, and the loopback bearer token.
//!
//! `bragi mcp` (and `bragi serve` for editor extensions) listen on loopback only and want a
//! bearer token from every client. The token lives in `<home>/serve/token` (32 random bytes as
//! hex, file mode 0600), made on first use, so one token works for every local API of a home and
//! `bragi serve token` or `bragi mcp token` prints it.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, ErrorKind, Result};

/// Bragi's data folder: `$BRAGI_HOME`, else `$XDG_DATA_HOME/bragi`, else `~/.local/share/bragi`
/// (the desktop app's), else `%APPDATA%\bragi`.
pub fn default_home() -> PathBuf {
    if let Some(h) = std::env::var_os("BRAGI_HOME") {
        return h.into();
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("bragi")
}

/// Where the local API token lives under a Bragi home.
pub fn token_path(home: &Path) -> PathBuf {
    home.join("serve").join("token")
}

fn storage(path: &Path, e: std::io::Error) -> CoreError {
    CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display()))
}

/// Reads the token, making one (32 random bytes, hex, file mode 0600) on first use.
pub fn load_or_create_token(home: &Path) -> Result<String> {
    let path = token_path(home);
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim();
        if t.len() >= 32 {
            return Ok(t.to_string());
        }
    }
    let dir = path.parent().unwrap_or(home);
    std::fs::create_dir_all(dir).map_err(|e| storage(dir, e))?;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| CoreError::new(ErrorKind::Other, format!("no randomness for the token: {e}")))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = dir.join(".token.tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| storage(&tmp, e))?;
        f.write_all(token.as_bytes()).map_err(|e| storage(&tmp, e))?;
        f.sync_all().map_err(|e| storage(&tmp, e))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| storage(&path, e))?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_made_once_and_private() {
        let d = tempfile::tempdir().unwrap();
        let t = load_or_create_token(d.path()).unwrap();
        assert_eq!(t.len(), 64);
        assert_eq!(load_or_create_token(d.path()).unwrap(), t);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(d.path())).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}

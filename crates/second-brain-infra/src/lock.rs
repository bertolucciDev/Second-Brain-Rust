//! Lock advisory cross-platform (G1–G3): uma conexão de escrita por processo.
//!
//! Usa `flock` no Linux/macOS e `LockFileEx` no Windows (fs4). O lock é adquirido
//! no arquivo `<db>.lock` e mantido enquanto o SqliteStore estiver aberto.

use std::fs::File;
use std::io;
use std::path::Path;

use fs4::fs_std::FileExt;

/// Guard de lock de arquivo. Drop libera o lock (RAII).
#[derive(Debug)]
pub struct FileLock {
    file: File,
}

impl FileLock {
    /// Adquire lock **exclusivo não-bloqueante** em `{db_path}.lock`.
    ///
    /// Retorna `Err` se outro processo detém o lock (evita dois escritores).
    pub fn acquire_for(db_path: &Path) -> io::Result<FileLock> {
        let lock_path = lock_path_for(db_path);
        let file = File::create(&lock_path)?;
        let acquired = file.try_lock_exclusive().map_err(|e| {
            io::Error::other(format!("failed to lock {}: {e}", lock_path.display()))
        })?;
        if !acquired {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "database is locked by another process (lock file: {})",
                    lock_path.display()
                ),
            ));
        }
        Ok(FileLock { file })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Caminho do arquivo de lock derivado do `dbPath` (`index.db` → `index.db.lock`).
pub fn lock_path_for(db_path: &Path) -> std::path::PathBuf {
    let mut os = db_path.as_os_str().to_owned();
    os.push(".lock");
    std::path::PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn lock_path_derives_db_path() {
        let p = lock_path_for(Path::new("/v/.memoryos/index.db"));
        assert_eq!(p, Path::new("/v/.memoryos/index.db.lock"));
    }

    /// Helper que roda como processo filho: segura o lock e sinaliza via marcador.
    #[test]
    fn lock_helper_holds() {
        let dir = std::env::var("LOCK_TEST_DIR").unwrap_or_default();
        if dir.is_empty() {
            return; // também roda no suite normalmente — no-op
        }
        let db = Path::new(&dir).join("index.db");
        let _guard = FileLock::acquire_for(&db).unwrap();
        fs_write(&Path::new(&dir).join("child_hold"), "1".as_bytes());
        let release = Path::new(&dir).join("release");
        while !release.exists() {
            thread::sleep(Duration::from_millis(20));
        }
        // hold mantém o lock até o fim do teste.
    }

    /// G1/G2: exclusividade do lock é *entre processos* (fcntl é per-process).
    /// Um processo-filho segura o lock; a tentativa no pai deve falhar, e conseguir
    /// após o release.
    #[test]
    fn cross_process_lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");

        let exe = std::env::current_exe().unwrap();
        let mut child = Command::new(&exe)
            .args(["lock::tests::lock_helper_holds", "--exact"])
            .env("LOCK_TEST_DIR", dir.path().as_os_str())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let hold = dir.path().join("child_hold");
        for _ in 0..200 {
            if hold.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(hold.exists(), "filho não segurou o lock a tempo");

        let err = FileLock::acquire_for(&db).unwrap_err();
        assert!(
            err.to_string().contains("locked"),
            "esperava lock ocupado pelo filho, got {err}"
        );

        fs_write(&dir.path().join("release"), b"1");
        let status = child.wait().unwrap();
        assert!(status.success());

        // liberado: o lock volta a ser adquirível
        let again = FileLock::acquire_for(&db).unwrap();
        drop(again);
    }

    fn fs_write(p: &Path, bytes: &[u8]) {
        use std::io::Write;
        let mut f = File::create(p).unwrap();
        f.write_all(bytes).unwrap();
    }
}

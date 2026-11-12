use crate::{
    domain::{ManagerError, Result},
    environment::private_directory,
};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

pub(crate) fn backup_database(source: &Path, destination: &Path, deadline: Instant) -> Result<()> {
    private_directory(destination.parent().ok_or(ManagerError::Rejected(
        "unsafe_path",
        "database has no destination directory",
    ))?)?;
    let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::ZERO)?;
    let mut destination = Connection::open(destination)?;
    {
        let backup = Backup::new(&source, &mut destination)?;
        let mut busy_waits = 0;
        loop {
            if Instant::now() >= deadline {
                return Err(ManagerError::Rejected(
                    "deadline_exceeded",
                    "database backup deadline exceeded",
                ));
            }
            match backup.step(128)? {
                StepResult::Done => break,
                StepResult::More => {}
                StepResult::Busy | StepResult::Locked => {
                    if busy_waits >= 20 {
                        return Err(ManagerError::Rejected(
                            "database_busy",
                            "database backup exhausted busy retries",
                        ));
                    }
                    busy_waits += 1;
                    thread::sleep(
                        Duration::from_millis(100)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                _ => {
                    return Err(ManagerError::Rejected(
                        "database_failure",
                        "unsupported SQLite backup result",
                    ));
                }
            }
        }
    }
    quick_check(&destination)?;
    destination.close().map_err(|(_, error)| error)?;
    Ok(())
}

fn quick_check(connection: &Connection) -> Result<()> {
    let result: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if result != "ok" {
        return Err(ManagerError::Rejected(
            "database_corrupt",
            "database quick_check failed",
        ));
    }
    Ok(())
}

pub(crate) fn validate_database(path: &Path) -> Result<()> {
    quick_check(&Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copies_live_wal_database_and_handles_quoted_path() {
        let directory = tempfile::tempdir().expect("database fixture");
        let source_path = directory.path().join("live 'state'.sqlite");
        let source = Connection::open(&source_path).expect("source database");
        source
            .execute_batch(
                "PRAGMA journal_mode=WAL; CREATE TABLE state(value); INSERT INTO state VALUES(42);",
            )
            .expect("WAL fixture");
        let destination = directory.path().join("backup.sqlite");
        backup_database(
            &source_path,
            &destination,
            Instant::now() + Duration::from_secs(5),
        )
        .expect("backup");
        let value: i64 = Connection::open(destination)
            .expect("snapshot")
            .query_row("SELECT value FROM state", [], |row| row.get(0))
            .expect("read row");
        assert_eq!(value, 42);
    }
    #[test]
    fn missing_source_is_not_created() {
        let directory = tempfile::tempdir().expect("fixture");
        let source = directory.path().join("absent.sqlite");
        assert!(
            backup_database(
                &source,
                &directory.path().join("backup.sqlite"),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(!source.exists());
    }
}

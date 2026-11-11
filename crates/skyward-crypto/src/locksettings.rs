//! What the phone's lock settings keep: a value under a name, for each
//! user. Among them is which of the protectors the phone keeps for a user
//! is the one that user's own credential opens.

use anyhow::{Context, Result};
use std::path::Path;

/// Where the phone keeps them.
pub const DATABASE: &str = "/data/system/locksettings.db";

/// The name the protector of a user's own secret is recorded under. Its
/// value is the number that protector's files are named after.
const CURRENT_PROTECTOR: &str = "sp-handle";

/// The number that names no protector. The files the phone keeps for a
/// user's secret itself, rather than for any one protector, are named
/// after it.
const NO_PROTECTOR: i64 = 0;

/// Which protector `user`'s own credential opens, named as its files are.
/// A phone that recorded none for this user gives nothing.
pub fn current_protector(database: &Path, user: u32) -> Result<Option<String>> {
    let Some(recorded) = value(database, CURRENT_PROTECTOR, user)? else { return Ok(None) };
    let id: i64 = recorded
        .parse()
        .with_context(|| format!("{} records user {user}'s protector as {recorded:?}", database.display()))?;
    Ok((id != NO_PROTECTOR).then(|| crate::user::protector_name(id)))
}

/// What the lock settings keep for `user` under `name`, where they keep
/// anything. The database is copied with its own log into a scratch
/// directory and read there: what the phone wrote last may still be in the
/// log, and a protector recorded just before it stopped is exactly what
/// would otherwise be missed.
fn value(database: &Path, name: &str, user: u32) -> Result<Option<String>> {
    let db = open_logged(database)?;
    let mut query = db
        .prepare("SELECT value FROM locksettings WHERE name = ?1 AND user = ?2")
        .context("ask the lock settings what they keep")?;
    let mut rows = query
        .query((name, user))
        .with_context(|| format!("ask the lock settings what {name} is for user {user}"))?;
    let Some(row) = rows.next().with_context(|| format!("read what {} says about {name}", database.display()))? else {
        return Ok(None);
    };
    row.get(0).with_context(|| format!("read what {name} is for user {user}"))
}

/// The database as it stood when the phone stopped, with the log beside it
/// folded in. The phone is not running, so nothing of its is written: the
/// database, `locksettings.db-wal` and `-shm` are copied into the temporary
/// directory, and the copy is opened and read there.
fn open_logged(database: &Path) -> Result<rusqlite::Connection> {
    let dir = std::env::temp_dir().join(format!("skyward-locksettings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("make {}", dir.display()))?;
    let at = dir.join("locksettings.db");
    std::fs::copy(database, &at).with_context(|| format!("copy {} to {}", database.display(), at.display()))?;
    for suffix in ["-wal", "-shm"] {
        let from = beside(database, suffix);
        if from.exists() {
            std::fs::copy(&from, beside(&at, suffix)).with_context(|| format!("copy {}", from.display()))?;
        }
    }
    rusqlite::Connection::open(&at).with_context(|| format!("open {}", at.display()))
}

/// `path` with `suffix` put after its whole name, as SQLite names its log.
fn beside(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    std::path::PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock settings as the phone writes them: the table it keeps, holding
    /// the rows given.
    fn scratch(test: &str, rows: &[(&str, u32, Option<&str>)]) -> std::path::PathBuf {
        let at = std::env::temp_dir().join(format!("skyward-locksettings-{test}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&at);
        let db = rusqlite::Connection::open(&at).unwrap();
        db.execute("CREATE TABLE locksettings (_id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, user INTEGER, value TEXT)", ()).unwrap();
        for (name, user, value) in rows {
            db.execute("INSERT INTO locksettings (name, user, value) VALUES (?1, ?2, ?3)", (name, user, value)).unwrap();
        }
        at
    }

    #[test]
    fn the_protector_recorded_for_a_user_is_the_one_named() {
        let at = scratch("recorded", &[
            ("sp-handle", 0, Some("623346955855960006")),
            ("sp-handle", 10, Some("-1")),
            ("lockscreen.password_type", 0, Some("65536")),
        ]);
        assert_eq!(current_protector(&at, 0).unwrap().as_deref(), Some("08a692c0f39d97c6"));
        assert_eq!(current_protector(&at, 10).unwrap().as_deref(), Some("ffffffffffffffff"));
        assert_eq!(current_protector(&at, 11).unwrap(), None, "a user the phone recorded nothing for");
        std::fs::remove_file(&at).unwrap();
    }

    #[test]
    fn the_number_of_no_protector_and_a_row_holding_nothing_give_none() {
        let at = scratch("none", &[("sp-handle", 0, Some("0")), ("sp-handle", 10, None)]);
        assert_eq!(current_protector(&at, 0).unwrap(), None);
        assert_eq!(current_protector(&at, 10).unwrap(), None, "a row holding nothing");
        std::fs::remove_file(&at).unwrap();
    }

    #[test]
    fn a_value_that_is_no_number_and_a_database_that_is_not_there_are_errors() {
        let at = scratch("odd", &[("sp-handle", 0, Some("behind the sofa"))]);
        assert!(current_protector(&at, 0).is_err());
        assert!(current_protector(Path::new("/nowhere/locksettings.db"), 0).is_err());
        std::fs::remove_file(&at).unwrap();
    }
}

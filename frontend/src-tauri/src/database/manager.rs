use sha2::{Digest, Sha384};
use sqlx::{migrate::MigrateDatabase, Result, Sqlite, SqlitePool, Transaction};
use std::fs;
use std::path::Path;

#[derive(Clone)]
pub struct DatabaseManager {
    pool: SqlitePool,
}

impl DatabaseManager {
    pub async fn new(tauri_db_path: &str, backend_db_path: &str) -> Result<Self> {
        if let Some(parent_dir) = Path::new(tauri_db_path).parent() {
            if !parent_dir.exists() {
                fs::create_dir_all(parent_dir).map_err(|e| sqlx::Error::Io(e))?;
            }
        }

        if !Path::new(tauri_db_path).exists() {
            if Path::new(backend_db_path).exists() {
                log::info!(
                    "Copying database from {} to {}",
                    backend_db_path,
                    tauri_db_path
                );
                fs::copy(backend_db_path, tauri_db_path).map_err(|e| sqlx::Error::Io(e))?;
            } else {
                log::info!("Creating database at {}", tauri_db_path);
                Sqlite::create_database(tauri_db_path).await?;
            }
        }

        let pool = SqlitePool::connect(tauri_db_path).await?;

        Self::repair_line_ending_checksums(&pool).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Self::recover_interrupted_summary_processes(&pool).await?;

        Ok(DatabaseManager { pool })
    }

    /// Reconcile stored migration checksums that differ only by line endings.
    ///
    /// sqlx checksums a migration's exact bytes, so a database written by a
    /// build whose `.sql` files had CRLF endings is rejected by a build that
    /// has the canonical LF bytes - "migration N was previously applied but has
    /// been modified" - even though the SQL is character-for-character the same
    /// statement. Which endings a given build saw depends on the developer's
    /// checkout, so neither side is authoritative and the user cannot fix it.
    ///
    /// Rewrite those checksums to match this build. This is deliberately not a
    /// blanket "skip the checksum" switch: a migration whose SQL genuinely
    /// changed hashes to neither line-ending variant, so it is left alone and
    /// sqlx still refuses to run it. Migrations stay immutable; only the
    /// representation of their line breaks is treated as insignificant.
    async fn repair_line_ending_checksums(pool: &SqlitePool) -> Result<()> {
        // A database that has never been migrated has no table to repair, and
        // `_sqlx_migrations` is created by the migrator itself further on.
        let Ok(applied) =
            sqlx::query_as::<_, (i64, Vec<u8>)>("SELECT version, checksum FROM _sqlx_migrations")
                .fetch_all(pool)
                .await
        else {
            return Ok(());
        };

        for migration in sqlx::migrate!("./migrations").iter() {
            let Some((_, stored)) = applied
                .iter()
                .find(|(version, _)| *version == migration.version)
            else {
                continue;
            };
            if stored.as_slice() == migration.checksum.as_ref() {
                continue;
            }
            if !line_ending_checksums(&migration.sql).contains(stored) {
                // Not a line-ending difference - the migration really was
                // edited after it shipped. Leave it for sqlx to reject.
                continue;
            }

            log::warn!(
                "Migration {} is recorded with a checksum that differs only in line endings; \
                 updating the stored checksum to match this build",
                migration.version
            );
            sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
                .bind(migration.checksum.as_ref())
                .bind(migration.version)
                .execute(pool)
                .await?;
        }

        Ok(())
    }

    // NOTE: So for the first time users they needs to start the application
    // after they can just delete the existing .sqlite file and then copy the existing .db file to
    // the current app dir, So the system detects legacy db and copy it and starts with that data
    // (Newly created .sqlite with the copied content from .db)
    pub async fn new_from_app_handle(app_handle: &tauri::AppHandle) -> Result<Self> {
        // Resolve the app's data directory
        let app_data_dir =
            crate::portable::app_data_dir(&app_handle).expect("failed to get app data dir");
        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Define database paths
        let tauri_db_path = app_data_dir
            .join("meeting_minutes.sqlite")
            .to_string_lossy()
            .to_string();
        // Legacy backend DB path (for auto-migration if exists)
        let backend_db_path = app_data_dir
            .join("meeting_minutes.db")
            .to_string_lossy()
            .to_string();

        // WAL file paths for defensive cleanup
        let wal_path = app_data_dir.join("meeting_minutes.sqlite-wal");
        let shm_path = app_data_dir.join("meeting_minutes.sqlite-shm");

        log::info!("Tauri DB path: {}", tauri_db_path);
        log::info!("Legacy backend DB path: {}", backend_db_path);

        // Try to open database with defensive WAL handling
        match Self::new(&tauri_db_path, &backend_db_path).await {
            Ok(db_manager) => {
                log::info!("Database opened successfully");
                Ok(db_manager)
            }
            Err(e) => {
                // Check if error is due to corrupted WAL file
                let error_msg = e.to_string();
                if error_msg.contains("malformed") || error_msg.contains("corrupt") {
                    log::warn!("Database appears corrupted, likely due to orphaned WAL file. Attempting recovery...");
                    log::warn!("Error details: {}", error_msg);

                    // Delete potentially corrupted WAL/SHM files
                    if wal_path.exists() {
                        match fs::remove_file(&wal_path) {
                            Ok(_) => log::info!("Removed orphaned WAL file: {:?}", wal_path),
                            Err(e) => log::warn!("Failed to remove WAL file: {}", e),
                        }
                    }
                    if shm_path.exists() {
                        match fs::remove_file(&shm_path) {
                            Ok(_) => log::info!("Removed orphaned SHM file: {:?}", shm_path),
                            Err(e) => log::warn!("Failed to remove SHM file: {}", e),
                        }
                    }

                    // Retry connection without WAL files
                    log::info!("Retrying database connection after WAL cleanup...");
                    match Self::new(&tauri_db_path, &backend_db_path).await {
                        Ok(db_manager) => {
                            log::info!("Database opened successfully after WAL recovery");
                            Ok(db_manager)
                        }
                        Err(retry_err) => {
                            log::error!(
                                "Database connection failed even after WAL cleanup: {}",
                                retry_err
                            );
                            Err(retry_err)
                        }
                    }
                } else {
                    // Not a WAL-related error, propagate original error
                    log::error!("Database connection failed: {}", error_msg);
                    Err(e)
                }
            }
        }
    }

    /// Create an empty database without importing the legacy `.db` file.
    /// Used only after the user explicitly chose database recovery.
    pub async fn new_empty_from_app_handle(app_handle: &tauri::AppHandle) -> Result<Self> {
        let app_data_dir =
            crate::portable::app_data_dir(app_handle).expect("failed to get app data dir");
        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(sqlx::Error::Io)?;
        }

        let tauri_db_path = app_data_dir
            .join("meeting_minutes.sqlite")
            .to_string_lossy()
            .to_string();

        if Path::new(&tauri_db_path).exists() {
            return Err(sqlx::Error::Protocol(
                "refusing to overwrite an existing database".to_string(),
            ));
        }

        log::info!("Creating fresh database at {}", tauri_db_path);
        Sqlite::create_database(&tauri_db_path).await?;
        let pool = SqlitePool::connect(&tauri_db_path).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Self::recover_interrupted_summary_processes(&pool).await?;

        Ok(DatabaseManager { pool })
    }

    /// Check if this is the first launch (sqlite database doesn't exist yet)
    pub async fn is_first_launch(app_handle: &tauri::AppHandle) -> Result<bool> {
        let app_data_dir =
            crate::portable::app_data_dir(&app_handle).expect("failed to get app data dir");

        let tauri_db_path = app_data_dir.join("meeting_minutes.sqlite");

        Ok(!tauri_db_path.exists())
    }

    /// Import a legacy database from the specified path and initialize
    pub async fn import_legacy_database(
        app_handle: &tauri::AppHandle,
        legacy_db_path: &str,
    ) -> Result<Self> {
        let app_data_dir =
            crate::portable::app_data_dir(&app_handle).expect("failed to get app data dir");

        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Copy legacy database to app data directory as meeting_minutes.db
        let target_legacy_path = app_data_dir.join("meeting_minutes.db");
        log::info!(
            "Copying legacy database from {} to {}",
            legacy_db_path,
            target_legacy_path.display()
        );

        fs::copy(legacy_db_path, &target_legacy_path).map_err(|e| sqlx::Error::Io(e))?;

        // Now use the standard initialization which will detect and migrate the legacy db
        Self::new_from_app_handle(app_handle).await
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    async fn recover_interrupted_summary_processes(pool: &SqlitePool) -> Result<()> {
        let recovered = crate::database::repositories::summary::SummaryProcessesRepository::fail_pending_processes_after_restart(pool).await?;
        if recovered > 0 {
            log::warn!(
                "Marked {} orphaned summary process(es) as failed after application restart",
                recovered
            );
        }
        Ok(())
    }

    pub async fn with_transaction<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Transaction<'_, Sqlite>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut tx = self.pool.begin().await?;
        let result = f(&mut tx).await;

        match result {
            Ok(val) => {
                tx.commit().await?;
                Ok(val)
            }
            Err(err) => {
                tx.rollback().await?;
                Err(err)
            }
        }
    }

    /// Cleanup database connection and checkpoint WAL
    /// This should be called on application shutdown to ensure:
    /// - All WAL changes are written to the main database file
    /// - The .wal and .shm files are deleted
    /// - Connection pool is gracefully closed
    pub async fn cleanup(&self) -> Result<()> {
        log::info!("Starting database cleanup...");

        // Force checkpoint of WAL to main database file and remove WAL file
        // TRUNCATE mode: checkpoints all pages AND deletes the WAL file
        match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
        {
            Ok(_) => log::info!("WAL checkpoint completed successfully"),
            Err(e) => log::warn!("WAL checkpoint failed (non-fatal): {}", e),
        }

        // Close the connection pool gracefully
        self.pool.close().await;
        log::info!("Database connection pool closed");

        Ok(())
    }
}

/// The two checksums the same SQL produces under either line-ending
/// convention. sqlx hashes the migration text verbatim, so these are the only
/// values a byte-for-byte equivalent migration can have been stored as.
fn line_ending_checksums(sql: &str) -> [Vec<u8>; 2] {
    let with_lf = sql.replace("\r\n", "\n");
    let with_crlf = with_lf.replace('\n', "\r\n");
    [
        Sha384::digest(with_lf.as_bytes()).to_vec(),
        Sha384::digest(with_crlf.as_bytes()).to_vec(),
    ]
}

#[cfg(test)]
mod line_ending_tests {
    use super::line_ending_checksums;
    use sha2::{Digest, Sha384};

    #[test]
    fn both_conventions_are_recognized() {
        let lf = "CREATE TABLE a (\n  id TEXT\n);\n";
        let crlf = "CREATE TABLE a (\r\n  id TEXT\r\n);\r\n";

        // Whichever form this build embedded, the other build's stored
        // checksum has to be recognized as equivalent.
        let from_lf = line_ending_checksums(lf);
        assert!(from_lf.contains(&Sha384::digest(lf.as_bytes()).to_vec()));
        assert!(from_lf.contains(&Sha384::digest(crlf.as_bytes()).to_vec()));
        assert_eq!(from_lf, line_ending_checksums(crlf));
    }

    #[test]
    fn edited_sql_is_not_recognized() {
        let original = "CREATE TABLE a (\n  id TEXT\n);\n";
        let edited = "CREATE TABLE a (\n  id INTEGER\n);\n";

        assert!(!line_ending_checksums(original)
            .contains(&Sha384::digest(edited.as_bytes()).to_vec()));
    }
}

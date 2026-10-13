use rusqlite::{Connection, OpenFlags};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

pub struct Image {
    pub path: PathBuf,
    pub uuid: Uuid,
}

impl Image {
    pub fn new(root: &Path, name: &str) -> Self {
        let tree = root.join(name);
        fs::create_dir(&tree).unwrap();
        if name == "seeded" {
            let connection = Connection::open(tree.join("state.db")).unwrap();
            connection
                .execute("CREATE TABLE preexisting_schema(value TEXT NOT NULL)", [])
                .unwrap();
            connection
                .execute(
                    "INSERT INTO preexisting_schema VALUES (?1)",
                    ["seed preserved"],
                )
                .unwrap();
        } else if name == "facts" {
            fs::write(tree.join("sentinel"), "read-only ordinary user\n").unwrap();
        }
        let path = root.join(format!("{name}.raw"));
        File::create(&path)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        let uuid = Uuid::new_v4();
        let result = Command::new("/usr/sbin/mkfs.ext4")
            .args([
                "-q",
                "-F",
                "-E",
                "lazy_itable_init=0,lazy_journal_init=0",
                "-U",
            ])
            .arg(uuid.to_string())
            .arg("-d")
            .arg(tree)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "mkfs: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        if name == "facts" {
            // Guest owner write permission makes EROFS distinguishable from
            // ordinary DAC denial; these changes happen before the RO snapshot.
            debugfs(&path, true, "set_inode_field /sentinel uid 10001");
            debugfs(&path, true, "set_inode_field /sentinel gid 10001");
        }
        Self { path, uuid }
    }

    pub fn assert_uuid(&self) {
        let mut file = File::open(&self.path).unwrap();
        file.seek(SeekFrom::Start(1024 + 104)).unwrap();
        let mut bytes = [0; 16];
        file.read_exact(&mut bytes).unwrap();
        assert_eq!(Uuid::from_bytes(bytes), self.uuid);
    }

    pub fn database_bytes(&self, name: &str) -> Vec<u8> {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join(name);
        let request = format!("dump /{name} {}", destination.display());
        debugfs(&self.path, false, &request);
        fs::read(destination).expect("database retained in actual ext4 image")
    }

    pub fn assert_database(&self, name: &str, query: &str, expected: &str) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("database");
        fs::write(&path, self.database_bytes(name)).unwrap();
        let connection =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let value: String = connection.query_row(query, [], |row| row.get(0)).unwrap();
        assert_eq!(value, expected);
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }
}

fn debugfs(path: &Path, write: bool, request: &str) {
    let mut command = Command::new("/usr/sbin/debugfs");
    if write {
        command.arg("-w");
    }
    let result = command.args(["-R", request]).arg(path).output().unwrap();
    assert!(
        result.status.success(),
        "debugfs: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

pub fn hash(path: &Path) -> String {
    let result = Command::new("sha256sum").arg(path).output().unwrap();
    assert!(result.status.success());
    String::from_utf8(result.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

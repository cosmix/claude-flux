use super::*;
use std::fs::{FileTimes, OpenOptions};
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

#[test]
fn write_index_skips_an_identical_file() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root).unwrap();
    write_index(root).unwrap();
    let path = root.join(INDEX_FILENAME);
    let fixed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_234_567);
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(fixed))
        .unwrap();
    let before = fs::metadata(&path).unwrap().modified().unwrap();

    write_index(root).unwrap();

    let after = fs::metadata(path).unwrap().modified().unwrap();
    assert_eq!(after, before);
}

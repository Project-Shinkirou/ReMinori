use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
pub trait StorageBackend {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), String>;
    fn remove(&self, key: &str) -> Result<(), String>;
}
#[derive(Default)]
pub struct MemoryStorage {
    values: RefCell<HashMap<String, Vec<u8>>>,
}
impl MemoryStorage {
    pub fn shared() -> Rc<Self> {
        Rc::new(Self::default())
    }
}
impl StorageBackend for MemoryStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.values.borrow().get(key).cloned())
    }
    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        self.values.borrow_mut().insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }
    fn remove(&self, key: &str) -> Result<(), String> {
        self.values.borrow_mut().remove(key);
        Ok(())
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub struct FileStorage {
    root: std::path::PathBuf,
}
#[cfg(not(target_arch = "wasm32"))]
impl FileStorage {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
    fn path(&self, key: &str) -> Result<std::path::PathBuf, String> {
        let path = std::path::Path::new(key);
        if key.is_empty() || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!("invalid storage key {key:?}"));
        }
        Ok(self.root.join(path))
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl StorageBackend for FileStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let path = self.path(key)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("read {}: {error}", path.display())),
        }
    }
    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.path(key)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, bytes)
            .map_err(|error| format!("write {}: {error}", path.display()))
    }
    fn remove(&self, key: &str) -> Result<(), String> {
        let path = self.path(key)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("remove {}: {error}", path.display())),
        }
    }
}


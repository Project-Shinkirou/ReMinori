use crate::vfs::{Image, Vfs};
use eden_formats::script::ScriptIr;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceGroup {
    Scr,
    Bg,
    St,
    Sys,
    Bgm,
    Voice,
    Se,
    Mov,
}
impl ResourceGroup {
    pub fn name(self) -> &'static str {
        match self {
            Self::Scr => "scr",
            Self::Bg => "bg",
            Self::St => "st",
            Self::Sys => "sys",
            Self::Bgm => "bgm",
            Self::Voice => "voice",
            Self::Se => "se",
            Self::Mov => "mov",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceState<T> {
    Ready(T),
    Pending,
    Missing,
    Failed(String),
}
pub struct ResourceRequestCache<K, T> {
    entries: RefCell<HashMap<K, ResourceState<T>>>,
}
impl<K, T> Default for ResourceRequestCache<K, T> {
    fn default() -> Self {
        Self {
            entries: RefCell::new(HashMap::new()),
        }
    }
}
impl<K, T> ResourceRequestCache<K, T>
where
    K: Clone + Eq + Hash,
    T: Clone,
{
    pub fn start_or_get(&self, key: K) -> (bool, ResourceState<T>) {
        if let Some(state) = self.entries.borrow().get(&key) {
            return (false, state.clone());
        }
        self.entries.borrow_mut().insert(key, ResourceState::Pending);
        (true, ResourceState::Pending)
    }
    pub fn finish(&self, key: K, state: ResourceState<T>) {
        assert!(state.is_terminal(), "resource completion must be terminal");
        self.entries.borrow_mut().insert(key, state);
    }
    pub fn retry(&self, key: &K) {
        let terminal = self
            .entries
            .borrow()
            .get(key)
            .is_some_and(ResourceState::is_terminal);
        if terminal {
            self.entries.borrow_mut().remove(key);
        }
    }
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
    }
}
impl<T> ResourceState<T> {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending)
    }
    pub fn as_ref(&self) -> ResourceState<&T> {
        match self {
            Self::Ready(value) => ResourceState::Ready(value),
            Self::Pending => ResourceState::Pending,
            Self::Missing => ResourceState::Missing,
            Self::Failed(error) => ResourceState::Failed(error.clone()),
        }
    }
    pub fn ready(self) -> Option<T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Pending | Self::Missing | Self::Failed(_) => None,
        }
    }
}
pub trait ResourceStore {
    fn request_bytes(&self, group: ResourceGroup, name: &str) -> ResourceState<Rc<[u8]>>;
    fn request_image(&self, name: &str) -> ResourceState<Rc<Image>>;
    fn script(&self, name: &str) -> Option<Rc<ScriptIr>>;
    fn pump(&self);
    fn invalidate_image(&self, _name: &str) {}
}
pub struct NativeResourceStore {
    vfs: Rc<Vfs>,
}
impl NativeResourceStore {
    pub fn new(vfs: Rc<Vfs>) -> Self {
        Self { vfs }
    }
    pub fn vfs(&self) -> &Rc<Vfs> {
        &self.vfs
    }
}
impl ResourceStore for NativeResourceStore {
    fn request_bytes(
        &self,
        group: ResourceGroup,
        name: &str,
    ) -> ResourceState<Rc<[u8]>> {
        self.vfs.request_bytes(group, name)
    }
    fn request_image(&self, name: &str) -> ResourceState<Rc<Image>> {
        self.vfs.request_image(name)
    }
    fn script(&self, name: &str) -> Option<Rc<ScriptIr>> {
        ResourceStore::script(self.vfs.as_ref(), name)
    }
    fn pump(&self) {}
    fn invalidate_image(&self, name: &str) {
        self.vfs.invalidate_image(name);
    }
}
impl ResourceStore for Vfs {
    fn request_bytes(
        &self,
        group: ResourceGroup,
        name: &str,
    ) -> ResourceState<Rc<[u8]>> {
        self.read_entry_debug(group.name(), name)
            .map(|bytes| ResourceState::Ready(Rc::from(bytes)))
            .unwrap_or(ResourceState::Missing)
    }
    fn request_image(&self, name: &str) -> ResourceState<Rc<Image>> {
        self.image(name).map(ResourceState::Ready).unwrap_or(ResourceState::Missing)
    }
    fn script(&self, name: &str) -> Option<Rc<ScriptIr>> {
        self.script(name)
    }
    fn pump(&self) {}
    fn invalidate_image(&self, name: &str) {
        self.invalidate_image(name);
    }
}


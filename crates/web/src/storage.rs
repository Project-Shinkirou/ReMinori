#[cfg(any(target_arch = "wasm32", test))]
fn object_store_key(key: &str) -> Result<(&'static str, String), String> {
    match key {
        "settings.tsv" => Ok(("settings", "current".into())),
        "system.dat" => Ok(("progress", "current".into())),
        "readflags.bin" => Ok(("readflags", "current".into())),
        "cgseen.tsv" => Ok(("cgseen", "current".into())),
        key if key.starts_with("saves/") && key.len() > "saves/".len() => {
            Ok(("saves", key["saves/".len()..].to_owned()))
        }
        _ => Err(format!("unsupported Web storage key {key:?}")),
    }
}
#[cfg(any(target_arch = "wasm32", test))]
fn public_key(store: &str, key: &str) -> Option<String> {
    match (store, key) {
        ("settings", "current") => Some("settings.tsv".into()),
        ("progress", "current") => Some("system.dat".into()),
        ("readflags", "current") => Some("readflags.bin".into()),
        ("cgseen", "current") => Some("cgseen.tsv".into()),
        ("saves", key) => Some(format!("saves/{key}")),
        _ => None,
    }
}
#[cfg(target_arch = "wasm32")]
mod platform {
    use super::{object_store_key, public_key};
    use eden_engine::StorageBackend;
    use js_sys::{Array, Promise, Reflect, Uint8Array};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::{spawn_local, JsFuture};
    thread_local! {
        static ACTIVE_STORAGE : RefCell < Option < Rc < IndexedDbStorage >>> = const {
        RefCell::new(None) };
    }
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = edenStorageLoadAll)]
        fn storage_load_all() -> Promise;
        #[wasm_bindgen(js_name = edenStoragePut)]
        fn storage_put(store: &str, key: &str, value: &JsValue) -> Promise;
        #[wasm_bindgen(js_name = edenStoragePutMany)]
        fn storage_put_many(store: &str, records: &JsValue) -> Promise;
        #[wasm_bindgen(js_name = edenStorageDelete)]
        fn storage_delete(store: &str, key: &str) -> Promise;
        #[wasm_bindgen(js_name = edenStorageClear)]
        fn storage_clear() -> Promise;
    }
    #[derive(Default)]
    pub struct IndexedDbStorage {
        mirror: RefCell<HashMap<String, Vec<u8>>>,
        last_error: Rc<RefCell<Option<String>>>,
    }
    impl IndexedDbStorage {
        async fn load() -> Result<Rc<Self>, String> {
            let value = JsFuture::from(storage_load_all()).await.map_err(js_error)?;
            let records = Array::from(&value);
            let storage = Rc::new(Self::default());
            for record in records.iter() {
                let store = string_property(&record, "store")?;
                let key = string_property(&record, "key")?;
                let Some(public) = public_key(&store, &key) else {
                    continue;
                };
                let value = Reflect::get(&record, &JsValue::from_str("value"))
                    .map_err(js_error)?;
                let array = Uint8Array::new(&value);
                let mut bytes = vec![0; array.length() as usize];
                array.copy_to(&mut bytes);
                storage.mirror.borrow_mut().insert(public, bytes);
            }
            Ok(storage)
        }
        async fn clear_all(&self) -> Result<(), String> {
            JsFuture::from(storage_clear()).await.map_err(js_error)?;
            self.mirror.borrow_mut().clear();
            *self.last_error.borrow_mut() = None;
            Ok(())
        }
        pub async fn write_durable(
            &self,
            key: &str,
            bytes: &[u8],
        ) -> Result<(), String> {
            let (store, object_key) = object_store_key(key)?;
            let value: JsValue = Uint8Array::from(bytes).into();
            JsFuture::from(storage_put(store, &object_key, &value))
                .await
                .map_err(js_error)?;
            self.mirror.borrow_mut().insert(key.to_owned(), bytes.to_vec());
            Ok(())
        }
        pub async fn write_save_pair(
            &self,
            save_name: &str,
            save_bytes: &[u8],
            thumbnail_bytes: &[u8],
        ) -> Result<(), String> {
            if save_name.contains(['/', '\\', ':', '\0'])
                || !save_name.to_ascii_lowercase().ends_with(".sav")
            {
                return Err(format!("不安全的存档文件名: {save_name:?}"));
            }
            let thumbnail_name = format!("{}.png", & save_name[..save_name.len() - 4]);
            let records = Array::new();
            for (key, bytes) in [
                (save_name, save_bytes),
                (thumbnail_name.as_str(), thumbnail_bytes),
            ] {
                let record = js_sys::Object::new();
                Reflect::set(&record, &JsValue::from_str("key"), &JsValue::from_str(key))
                    .map_err(js_error)?;
                let value: JsValue = Uint8Array::from(bytes).into();
                Reflect::set(&record, &JsValue::from_str("value"), &value)
                    .map_err(js_error)?;
                records.push(&record);
            }
            JsFuture::from(storage_put_many("saves", records.as_ref()))
                .await
                .map_err(js_error)?;
            let mut mirror = self.mirror.borrow_mut();
            mirror.insert(format!("saves/{save_name}"), save_bytes.to_vec());
            mirror.insert(format!("saves/{thumbnail_name}"), thumbnail_bytes.to_vec());
            Ok(())
        }
        pub fn save_entries(&self) -> Vec<(String, Vec<u8>)> {
            let mut entries: Vec<_> = self
                .mirror
                .borrow()
                .iter()
                .filter_map(|(key, bytes)| {
                    key.strip_prefix("saves/")
                        .map(|name| (name.to_owned(), bytes.clone()))
                })
                .collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            entries
        }
        pub fn has_save(&self, save_name: &str) -> bool {
            self.mirror.borrow().contains_key(&format!("saves/{save_name}"))
        }
    }
    impl StorageBackend for IndexedDbStorage {
        fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            object_store_key(key)?;
            Ok(self.mirror.borrow().get(key).cloned())
        }
        fn write(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
            let (store, object_key) = object_store_key(key)?;
            self.mirror.borrow_mut().insert(key.to_owned(), bytes.to_vec());
            let value: JsValue = Uint8Array::from(bytes).into();
            let public_key = key.to_owned();
            let last_error = Rc::clone(&self.last_error);
            spawn_local(async move {
                if let Err(error) = JsFuture::from(
                        storage_put(store, &object_key, &value),
                    )
                    .await
                {
                    let message = format!(
                        "IndexedDB 写入失败 {public_key}: {}", js_error(error)
                    );
                    *last_error.borrow_mut() = Some(message.clone());
                    report_error(&message);
                }
            });
            Ok(())
        }
        fn remove(&self, key: &str) -> Result<(), String> {
            let (store, object_key) = object_store_key(key)?;
            self.mirror.borrow_mut().remove(key);
            let public_key = key.to_owned();
            let last_error = Rc::clone(&self.last_error);
            spawn_local(async move {
                if let Err(error) = JsFuture::from(storage_delete(store, &object_key))
                    .await
                {
                    let message = format!(
                        "IndexedDB 删除失败 {public_key}: {}", js_error(error)
                    );
                    *last_error.borrow_mut() = Some(message.clone());
                    report_error(&message);
                }
            });
            Ok(())
        }
    }
    pub async fn initialize() -> Result<Rc<IndexedDbStorage>, String> {
        let storage = IndexedDbStorage::load().await?;
        ACTIVE_STORAGE.with(|active| *active.borrow_mut() = Some(Rc::clone(&storage)));
        Ok(storage)
    }
    pub async fn clear_active() -> Result<(), String> {
        let active = ACTIVE_STORAGE.with(|storage| storage.borrow().clone());
        if let Some(storage) = active {
            storage.clear_all().await
        } else {
            JsFuture::from(storage_clear()).await.map(|_| ()).map_err(js_error)
        }
    }
    fn string_property(value: &JsValue, name: &str) -> Result<String, String> {
        Reflect::get(value, &JsValue::from_str(name))
            .map_err(js_error)?
            .as_string()
            .ok_or_else(|| format!("IndexedDB record.{name} 不是字符串"))
    }
    fn report_error(message: &str) {
        web_sys::console::error_1(&JsValue::from_str(message));
        if let Some(status) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("status"))
        {
            status.set_class_name("status status--error");
            status.set_text_content(Some(message));
        }
    }
    fn js_error(value: JsValue) -> String {
        value
            .as_string()
            .or_else(|| {
                Reflect::get(&value, &JsValue::from_str("message")).ok()?.as_string()
            })
            .or_else(|| js_sys::JSON::stringify(&value).ok()?.as_string())
            .unwrap_or_else(|| "未知 IndexedDB 错误".into())
    }
}
#[cfg(target_arch = "wasm32")]
pub use platform::{clear_active, initialize, IndexedDbStorage};


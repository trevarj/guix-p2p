use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct CatalogItem {
    pub hash_part: String,
    pub store_path: Option<String>,
    pub nar_size: Option<u64>,
    pub nar_hash: Option<String>,
    pub p2p_available: bool,
}

pub fn upsert_catalog_item(
    catalog: &Arc<Mutex<HashMap<String, CatalogItem>>>,
    hash_part: String,
    store_path: Option<String>,
    nar_size: Option<u64>,
    nar_hash: Option<String>,
    p2p_available: bool,
) {
    let mut cat = catalog.lock().unwrap();
    cat.entry(hash_part.clone())
        .and_modify(|entry| {
            if store_path.is_some() {
                entry.store_path = store_path.clone();
            }
            if nar_size.is_some() {
                entry.nar_size = nar_size;
            }
            if nar_hash.is_some() {
                entry.nar_hash = nar_hash.clone();
            }
            entry.p2p_available |= p2p_available;
        })
        .or_insert_with(|| CatalogItem {
            hash_part,
            store_path,
            nar_size,
            nar_hash,
            p2p_available,
        });
}

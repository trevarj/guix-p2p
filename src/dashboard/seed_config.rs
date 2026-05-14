use std::{
    env, fs,
    path::{Path as FsPath, PathBuf},
};

use toml_edit::{Array, DocumentMut, Item, Value};

pub fn user_config_path() -> PathBuf {
    if let Ok(dir) = env::var("XDG_CONFIG_HOME") {
        PathBuf::from(dir).join("guix-p2p/config.toml")
    } else {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".config/guix-p2p/config.toml")
    }
}

pub fn persist_seed_path_to_config(store_path: &str, config_path: &FsPath) -> anyhow::Result<()> {
    let parent =
        config_path.parent().ok_or_else(|| anyhow::anyhow!("config path has no parent"))?;
    fs::create_dir_all(parent)?;

    let content = fs::read_to_string(config_path).unwrap_or_default();
    let mut doc = content.parse::<DocumentMut>().unwrap_or_else(|_| DocumentMut::new());
    let mut values = seed_paths(&doc);
    if !values.iter().any(|value| value == store_path) {
        values.push(store_path.to_string());
    }

    write_seed_paths(&mut doc, values);
    fs::write(config_path, doc.to_string())?;
    Ok(())
}

pub fn remove_seed_path_from_config(store_path: &str, config_path: &FsPath) -> anyhow::Result<()> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(_) => return Ok(()),
    };
    let mut doc = content.parse::<DocumentMut>().unwrap_or_else(|_| DocumentMut::new());
    let Some(existing) = doc.get("seed_paths").and_then(Item::as_array) else {
        return Ok(());
    };
    let values = existing
        .iter()
        .filter_map(Value::as_str)
        .filter(|value| *value != store_path)
        .map(str::to_string)
        .collect();

    write_seed_paths(&mut doc, values);
    fs::write(config_path, doc.to_string())?;
    Ok(())
}

fn seed_paths(doc: &DocumentMut) -> Vec<String> {
    doc.get("seed_paths")
        .and_then(Item::as_array)
        .map(|existing| existing.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn write_seed_paths(doc: &mut DocumentMut, values: Vec<String>) {
    let mut array = Array::default();
    values.into_iter().for_each(|value| {
        array.push(value);
    });
    doc["seed_paths"] = Item::Value(Value::Array(array));
}

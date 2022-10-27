use std::any::Any;
use anyhow::{anyhow, Context};
use simplelog::trace;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;
use std::path::Path;
use crate::file_system::File;
use crate::FileSystem;

pub trait Mergable {
    fn merge(&self, other: Self) -> anyhow::Result<Self>
    where
        Self: Sized;
}

// fn try_get_maps(file: File) -> anyhow::Result<(BTreeMap<String, dyn Any>, BTreeMap<String, _>)> {
//     let extension = file.source.extension()?.to_str()?;
//
//     match (extension) {
//         "conf" => Ok("hocon".to_string()),
//         "toml" => Ok("toml".to_string()),
//         "json" => Ok("json".to_string()),
//         "yaml" => Ok("yaml".to_string()),
//         "yml" => Ok("yaml".to_string()),
//         _ => Err(anyhow!("Unknown file type")),
//     }
// }

// impl<V> Mergable for Vec<V>
// where
//     V: Hash,
//     V: Eq
// {
//     // fn merge(&self, other: Self) -> anyhow::Result<Self> {
//     //     let mut set = HashSet::from_iter(self.clone().iter().clone());
//     //
//     //     // set.extend(other);
//     //     Ok(set.into_iter().collect_vec())
//     // }
// }

// impl<T> Merger<Vec<T>> for T
// where
//     T: Eq,
//     T: Clone,
// {
//     fn merge(existing: &mut Vec<T>, new: &Vec<T>) -> anyhow::Result<()> {
//         for item in new {
//             if existing.contains(item) {
//                 continue;
//             }
//
//             existing.push(item.clone());
//         }
//
//         Ok(())
//     }
// }
//
// impl Merger<HashMap<(), ()>> for HashMap<(), ()> {
//     fn merge<'a>(existing: &mut HashMap<(), ()>, new: &HashMap<(), ()>) -> anyhow::Result<()> {
//         existing.extend(new.into_iter().map(|(k, v)| (k.clone(), v.clone())));
//
//         Ok(())
//     }
// }
//
// impl Merger<Table> for Table {
//     fn merge(existing: &mut Table, new: &Table) -> anyhow::Result<()> {
//         for (key, value) in new.iter() {
//             if value.is_table() {
//                 let existing_table = existing.entry(key).or_insert(value.clone());
//                 if existing_table.is_table() {
//                     Merger::<Table>::merge(
//                         existing_table
//                             .as_table_mut()
//                             .context("Get existing as mut table")?,
//                         value.as_table().context("Get new as table")?,
//                     )?;
//                 }
//             } else if value.is_array() {
//                 let existing_array = existing.entry(key).or_insert(value.clone());
//                 if existing_array.is_array() {
//                     Merger::<Array>::merge(
//                         existing_array
//                             .as_array_mut()
//                             .context("Get existing as mut array")?,
//                         value.as_array().context("Get new as array")?,
//                     )?;
//                 }
//             } else {
//                 existing.insert(key, value.clone());
//             }
//         }
//
//         Ok(())
//     }
// }
//
// impl Merger<Document> for Document {
//     fn merge(existing: &mut Document, new: &Document) -> anyhow::Result<()> {
//         for (key, value) in new.iter() {
//             if existing.contains_key(key) {
//                 trace!("Merging key {}", key);
//                 let existing_value = existing.get_mut(key).unwrap();
//                 if existing_value.is_table() && value.is_table() {
//                     Merger::merge(
//                         existing_value.as_table_mut().unwrap(),
//                         value.as_table().unwrap(),
//                     )?;
//                 } else if existing_value.is_array() && value.is_array() {
//                     Merger::merge(
//                         existing_value.as_array_mut().unwrap(),
//                         value.as_array().unwrap(),
//                     )?;
//                 } else {
//                     *existing_value = value.clone();
//                 }
//             }
//         }
//
//         Ok(())
//     }
// }

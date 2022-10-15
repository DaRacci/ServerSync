// trait Merger<T> {
//     fn merge(existing: &mut T, new: &T) -> anyhow::Result<()>;
// }
//
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

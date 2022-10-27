use merge_yaml_hash::{MergeYamlHash, Yaml};
use std::collections::BTreeMap;
use std::{clone, mem, string};
use std::process::exit;
use toml::map::Entry;
use toml::value::Table;
use toml::{toml, Value};
use yaml_rust::parser::Parser;
use yaml_rust::{ScanError, YamlEmitter, YamlLoader};

/// YAML Hash with merge/update capabilities
///
/// Wrapper around `yaml_rust::yaml::Hash`, which is a type alias for
/// `linked_hash_map::LinkedHashMap`
#[derive(Debug)]
pub struct MergeTomlHash {
    pub data: Table,
}

pub struct TomlLoader<'a> {
    docs: Vec<Table>,
    // states
    // (current node, anchor_id) tuple
    doc_stack: Vec<(Entry<'a>, usize)>,
    key_stack: Vec<Entry<'a>>,
    anchor_map: BTreeMap<usize, Value>,
}

impl TomlLoader<'_> {
    fn insert_new_node(&mut self, node: (Entry, usize)) {
        // valid anchor id starts from 1
        if node.1 > 0 {
            self.anchor_map.insert(node.1, node.0.or_insert_with(|| exit(1)).clone());
        }
        if self.doc_stack.is_empty() {
            self.doc_stack.push(node);
        } else {
            let parent = self.doc_stack.last_mut().unwrap();
            match *parent {
                (Value::Array(ref mut v), _) => {
                    v.push(node.0.or_insert_with(|| exit(1)).clone());
                    // match *node.0 {
                    //     (Entry::Occupied(ref mut e)) => v.push(e.get().clone()),
                    //     (Entry::Vacant(ref mut e)) => unreachable!(),
                    // }
                }
                (Value::Table(ref mut t), _) => {
                    let cur_key = self.key_stack.last_mut().unwrap();

                    match *cur_key {
                        (Entry::Occupied(ref mut e)) => {
                            let mut new_key = Value::BadValue????;
                            mem::swap(&mut new_key, cur_key);
                            t.insert(new_key, node.0.or_insert_with(|| exit(1)).clone());
                        }
                        (Entry::Vacant(ref mut e)) => {
                            *cur_key = node.0;
                        }
                        _ => exit(1),
                    }
                }
                _ => exit(1),
            }
        }
    }

    pub fn load_from_str(source: &str) -> Result<Vec<Value>, ScanError> {
        let mut loader = TomlLoader {
            docs: vec![],
            doc_stack: vec![],
            key_stack: vec![],
            anchor_map: BTreeMap::new(),
        };

        let mut parser = toml::Value::from(source.chars());
        parser.load(&mut loader, true)?;
        Ok(loader.docs)
    }
}

impl MergeTomlHash {
    pub fn new() -> Box<MergeTomlHash> {
        Box::new(MergeTomlHash { data: Table::new() })
    }

    fn to_string(&self) -> String {
        let toml = toml::Value::from(self.data.clone());
        toml.to_string()
    }

    pub fn merge(&mut self, file_or_str: &str) {
        let path = std::path::Path::new(&file_or_str);
        let toml: String;
        if path.is_file() {
            toml = std::fs::read_to_string(&path).unwrap();
        } else {
            toml = file_or_str.to_string();
        }
        for doc in TomlLoader::load_from_str(&toml).unwrap() {
            if let Value::Table(h) = doc {
                self.data = self.merge_hashes(&self.data, &h);
            }
        }
    }

    fn merge_hashes(&self, a: &Table, b: &Table) -> Table {
        let mut r = a.clone();
        for (k, v) in b.iter() {
            if let Value::Table(bh) = v {
                if let Entry::Occupied(e) = r.entry(k.clone()) {
                    if let Value::Table(mut rh) = e.get().clone() {
                        rh = self.merge_hashes(&rh, bh);
                        r.insert(k.clone(), Value::Table(rh));
                        continue;
                    }
                }
            }
            r.insert(k.clone(), v.clone());
        }
        r
    }

    pub fn merge_vec(&mut self, files_or_strings: Vec<String>) {
        for file_or_string in files_or_strings {
            self.merge(&file_or_string);
        }
    }
}

impl std::fmt::Display for MergeTomlHash {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

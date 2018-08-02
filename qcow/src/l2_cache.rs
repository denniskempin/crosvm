// Copyright 2018 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std;
use std::collections::HashMap;

#[derive(Debug)]
pub enum Error {
    InvalidVectorLength,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct L2Table {
    cluster_addrs: Vec<u64>,
    dirty: bool,
}

impl L2Table {
    fn new(table_size: usize) -> L2Table {
        L2Table {
            cluster_addrs: vec![0, table_size as u64],
            dirty: true,
        }
    }

    fn from_vec(addrs: Vec<u64>) -> L2Table {
        L2Table {
            cluster_addrs: addrs,
            dirty: false,
        }
    }

    pub fn get(&self, index: usize) -> u64 {
        *self.cluster_addrs.get(index).unwrap_or(&0)
    }

    pub fn set(&mut self, index: usize, val: u64) {
        if index < self.cluster_addrs.len() {
            self.cluster_addrs[index] = val;
            self.dirty = true;
        }
    }

    pub fn addrs(&self) -> &Vec<u64> {
        &self.cluster_addrs
    }
}

#[derive(Debug)]
pub struct L2Cache {
    tables: HashMap<usize, L2Table>,
    table_size: usize,
}

impl L2Cache {
    pub fn new(table_size: usize, capacity: usize) -> L2Cache {
        L2Cache {
            tables: HashMap::with_capacity(capacity),
            table_size,
        }
    }
    
    pub fn get_table(&self, l1_index: usize) -> Option<&L2Table> {
        self.tables.get(&l1_index)
    }

    pub fn create_table(&self) -> L2Table {
        L2Table::new(self.table_size)
    }

    pub fn take_table(&mut self, l1_index: usize) -> Option<L2Table> {
        self.tables.remove(&l1_index).map(|mut t| {t.dirty = true; t})
    }

    pub fn insert(&mut self, l1_index: usize, table: L2Table) -> Option<L2Table> {
        let evicted = if self.tables.len() == self.tables.capacity() {
            // TODO(dgreid) smarter eviction
            let k = self.tables.keys().nth(0).unwrap().clone();
            self.tables.remove(&k)
        } else {
            None
        };

        self.tables.insert(l1_index, table);

        evicted
    }

    pub fn insert_vec(&mut self, l1_index: usize, addrs: Vec<u64>) -> Result<Option<L2Table>> {
        if addrs.len() != self.table_size {
            return Err(Error::InvalidVectorLength);
        }

        Ok(self.insert(l1_index, L2Table::from_vec(addrs)))
    }

    pub fn dirty_iter_mut(&mut self) -> impl Iterator<Item = &L2Table> {
        self.tables.iter().filter_map(|(k, v)| if v.dirty { Some(v) } else { None })
    }
}
#![allow(dead_code)]
//! Test helpers: a tiny history builder and a splitmix64 generator (the repo has no property-testing crate).

use hairspring_check::{Completeness, History, OpRecord, OpType};

/// Builds records with automatically increasing sequence numbers.
#[derive(Default)]
pub struct Hb {
    records: Vec<OpRecord>,
    seq: u64,
}

impl Hb {
    pub fn new() -> Self {
        Hb::default()
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        client: &str,
        id: u64,
        kind: OpType,
        f: &str,
        key: &str,
        value: Option<i64>,
        expect: Option<i64>,
    ) {
        self.seq += 1;
        self.records.push(OpRecord {
            client: client.to_string(),
            id,
            kind,
            f: f.to_string(),
            key: key.to_string(),
            value,
            expect,
            seq: self.seq,
        });
    }

    pub fn invoke(
        &mut self,
        client: &str,
        id: u64,
        f: &str,
        key: &str,
        value: Option<i64>,
    ) -> &mut Self {
        self.push(client, id, OpType::Invoke, f, key, value, None);
        self
    }

    pub fn invoke_cas(
        &mut self,
        client: &str,
        id: u64,
        key: &str,
        expect: i64,
        new: i64,
    ) -> &mut Self {
        self.push(
            client,
            id,
            OpType::Invoke,
            "cas",
            key,
            Some(new),
            Some(expect),
        );
        self
    }

    pub fn ok(
        &mut self,
        client: &str,
        id: u64,
        f: &str,
        key: &str,
        value: Option<i64>,
    ) -> &mut Self {
        self.push(client, id, OpType::Ok, f, key, value, None);
        self
    }

    pub fn fail(&mut self, client: &str, id: u64, f: &str, key: &str) -> &mut Self {
        self.push(client, id, OpType::Fail, f, key, None, None);
        self
    }

    pub fn info(&mut self, client: &str, id: u64, f: &str, key: &str) -> &mut Self {
        self.push(client, id, OpType::Info, f, key, None, None);
        self
    }

    /// Invoke then immediately complete ok (a sequential operation).
    pub fn seq_ok(
        &mut self,
        client: &str,
        id: u64,
        f: &str,
        key: &str,
        arg: Option<i64>,
        result: Option<i64>,
    ) -> &mut Self {
        self.invoke(client, id, f, key, arg);
        self.ok(client, id, f, key, result)
    }

    pub fn records(&self) -> &[OpRecord] {
        &self.records
    }

    pub fn build(&self) -> History {
        History::from_records(&self.records, Completeness::Complete).expect("valid history")
    }
}

/// splitmix64.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

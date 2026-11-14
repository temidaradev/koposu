//! Just enough of the Realm file format to read osu!lazer's `client.realm`:
//! tables, their columns, and the strings, integers, booleans, links and
//! lists of links in their objects. Read-only, and from the bytes of the
//! whole file, so a lazer running at the same time is never disturbed.
//!
//! A Realm file is a tree of arrays. Every array starts with an eight byte
//! header: four bytes of checksum, one of flags and element width, and a
//! 24-bit big-endian element count. A ref is a byte offset into the file; in
//! an array holding refs, an odd value is an integer shifted left by one.

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn error<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}

const HEADER: usize = 8;
const FILE_HEADER: usize = 24;

/// Column types, as Realm numbers them.
pub mod kind {
    pub const INT: u8 = 0;
    pub const BOOL: u8 = 1;
    pub const STRING: u8 = 2;
    pub const DOUBLE: u8 = 10;
    pub const LINK: u8 = 12;
}

const ATTR_NULLABLE: u8 = 16;
const ATTR_LIST: u8 = 32;
const ATTR_DICTIONARY: u8 = 64;
const ATTR_SET: u8 = 128;

#[derive(Clone, Copy)]
struct Node<'a> {
    bytes: &'a [u8],
    inner: bool,
    has_refs: bool,
    context: bool,
    width_type: u8,
    width: usize,
    len: usize,
    data: &'a [u8],
}

impl<'a> Node<'a> {
    fn get(&self, index: usize) -> i64 {
        if index >= self.len {
            return 0;
        }
        match self.width {
            0 => 0,
            1 | 2 | 4 => {
                let bit = index * self.width;
                let byte = self.data.get(bit / 8).copied().unwrap_or_default();
                i64::from((byte >> (bit % 8)) & ((1u8 << self.width) - 1))
            }
            8 => i64::from(self.data[index] as i8),
            16 => i64::from(i16::from_le_bytes(self.chunk(index))),
            32 => i64::from(i32::from_le_bytes(self.chunk(index))),
            _ => i64::from_le_bytes(self.chunk(index)),
        }
    }

    /// Element `index` of an array of unsigned integers, as object keys are.
    fn unsigned(&self, index: usize) -> i64 {
        match self.width {
            8 => i64::from(self.data.get(index).copied().unwrap_or_default()),
            16 => i64::from(u16::from_le_bytes(self.chunk(index))),
            32 => i64::from(u32::from_le_bytes(self.chunk(index))),
            _ => self.get(index),
        }
    }

    /// Element `index` of an array of floating point numbers, which are as
    /// wide as their type in bytes.
    fn float(&self, index: usize) -> Option<f64> {
        match (self.width_type, self.width) {
            (1, 8) | (0, 64) => Some(f64::from_le_bytes(self.chunk(index))),
            (1, 4) | (0, 32) => Some(f64::from(f32::from_le_bytes(self.chunk(index)))),
            _ => None,
        }
    }

    fn chunk<const N: usize>(&self, index: usize) -> [u8; N] {
        let start = index * N;
        self.data
            .get(start..start + N)
            .and_then(|slice| slice.try_into().ok())
            .unwrap_or([0; N])
    }

    /// Element `index` as a ref, or `None` for a tagged integer or nothing.
    fn child(&self, index: usize) -> Option<usize> {
        let value = self.get(index);
        (value > 0 && value & 1 == 0).then_some(value as usize)
    }

    /// Element `index` as a tagged integer.
    fn tagged(&self, index: usize) -> Option<i64> {
        let value = self.get(index);
        (value & 1 == 1).then_some(value >> 1)
    }

    fn values(&self) -> impl Iterator<Item = i64> + '_ {
        (0..self.len).map(|index| self.get(index))
    }
}

pub struct Realm<'a> {
    bytes: &'a [u8],
    top: usize,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub kind: u8,
    attrs: u8,
    /// Where its values sit in a cluster, after the keys.
    leaf: usize,
}

impl Column {
    pub fn is_list(&self) -> bool {
        self.attrs & ATTR_LIST != 0
    }

    fn is_collection(&self) -> bool {
        self.attrs & (ATTR_LIST | ATTR_DICTIONARY | ATTR_SET) != 0
    }

    fn nullable(&self) -> bool {
        self.attrs & ATTR_NULLABLE != 0
    }
}

pub struct Table<'r, 'a> {
    realm: &'r Realm<'a>,
    pub name: String,
    pub columns: Vec<Column>,
    root: Option<usize>,
}

/// One object: the cluster leaf holding it and its row there.
#[derive(Clone, Copy)]
pub struct Obj<'a> {
    pub key: i64,
    cluster: Node<'a>,
    row: usize,
}

impl<'a> Realm<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FILE_HEADER || &bytes[16..20] != b"T-DB" {
            return error("not a Realm file");
        }
        let select = usize::from(bytes[23] & 1);
        let offset = select * 8;
        let top = u64::from_le_bytes(
            bytes[offset..offset + 8]
                .try_into()
                .map_err(|_| Error("truncated header".into()))?,
        ) as usize;
        if top == 0 {
            // A file written in streaming form keeps its top ref in a footer.
            let footer = bytes
                .len()
                .checked_sub(16)
                .ok_or_else(|| Error("truncated footer".into()))?;
            let top = u64::from_le_bytes(
                bytes[footer..footer + 8]
                    .try_into()
                    .map_err(|_| Error("truncated footer".into()))?,
            ) as usize;
            return Ok(Self { bytes, top });
        }
        Ok(Self { bytes, top })
    }

    fn node(&self, at: usize) -> Result<Node<'a>> {
        let header = self
            .bytes
            .get(at..at + HEADER)
            .ok_or_else(|| Error(format!("ref {at} is outside the file")))?;
        let flags = header[4];
        let width = (1usize << (flags & 0x07)) >> 1;
        let width_type = (flags & 0x18) >> 3;
        let len =
            (usize::from(header[5]) << 16) | (usize::from(header[6]) << 8) | usize::from(header[7]);
        let size = match width_type {
            0 => (len * width).div_ceil(8),
            1 => len * width,
            2 => len,
            _ => {
                return error(format!(
                    "array at {at} is compressed, which this reader does not read"
                ));
            }
        };
        let data = self
            .bytes
            .get(at + HEADER..at + HEADER + size)
            .ok_or_else(|| Error(format!("array at {at} runs past the file")))?;
        Ok(Node {
            bytes: self.bytes,
            inner: flags & 0x80 != 0,
            has_refs: flags & 0x40 != 0,
            context: flags & 0x20 != 0,
            width_type,
            width,
            len,
            data,
        })
    }

    fn top(&self) -> Result<Node<'a>> {
        self.node(self.top)
    }

    /// Every table's name, as stored (`class_` and the class name).
    pub fn table_names(&self) -> Result<Vec<String>> {
        let top = self.top()?;
        let names = top.child(0).ok_or_else(|| Error("no table names".into()))?;
        self.strings(names)
    }

    pub fn table<'r>(&'r self, name: &str) -> Result<Table<'r, 'a>> {
        let names = self.table_names()?;
        let index = names
            .iter()
            .position(|stored| stored == name || stored.strip_prefix("class_") == Some(name))
            .ok_or_else(|| Error(format!("no table {name}")))?;
        let tables = self.node(
            self.top()?
                .child(1)
                .ok_or_else(|| Error("no tables".into()))?,
        )?;
        let top = self.node(
            tables
                .child(index)
                .ok_or_else(|| Error(format!("table {name} has no data")))?,
        )?;
        let spec = self.node(top.child(0).ok_or_else(|| Error("no spec".into()))?)?;
        let names = self.strings(
            spec.child(1)
                .ok_or_else(|| Error("no column names".into()))?,
        )?;
        let keys = spec
            .child(5)
            .map(|at| self.node(at))
            .transpose()?
            .ok_or_else(|| Error("no column keys".into()))?;
        let columns = keys
            .values()
            .enumerate()
            .map(|(index, key)| Column {
                name: names.get(index).cloned().unwrap_or_default(),
                kind: ((key >> 16) & 0x3f) as u8,
                attrs: ((key >> 22) & 0xff) as u8,
                leaf: (key & 0xffff) as usize + 1,
            })
            .collect();
        Ok(Table {
            realm: self,
            name: name.to_string(),
            columns,
            root: top.child(2),
        })
    }

    /// A string array in any of its three forms, or a B+tree of them.
    fn strings(&self, at: usize) -> Result<Vec<String>> {
        let node = self.node(at)?;
        let mut out = Vec::with_capacity(node.len);
        if node.inner {
            for index in 1..node.len.saturating_sub(1) {
                if let Some(child) = node.child(index) {
                    out.extend(self.strings(child)?);
                }
            }
            return Ok(out);
        }
        for index in 0..node.len {
            out.push(self.string_at(node, index)?.unwrap_or_default());
        }
        Ok(out)
    }

    fn string_at(&self, leaf: Node<'a>, index: usize) -> Result<Option<String>> {
        if !leaf.has_refs {
            // Short strings, each padded to the width; the last byte says how
            // much is padding, and the whole width means none at all.
            if leaf.width == 0 {
                return Ok(Some(String::new()));
            }
            let start = index * leaf.width;
            let Some(slot) = leaf.data.get(start..start + leaf.width) else {
                return Ok(None);
            };
            let padding = usize::from(slot[leaf.width - 1]);
            if padding >= leaf.width {
                return Ok(None);
            }
            let len = leaf.width - 1 - padding;
            return Ok(Some(String::from_utf8_lossy(&slot[..len]).into_owned()));
        }
        if leaf.context {
            // Long strings, each a blob of its own ending in a zero byte.
            let Some(blob) = leaf.child(index) else {
                return Ok(None);
            };
            let blob = self.node(blob)?;
            let text = blob.data.strip_suffix(&[0]).unwrap_or(blob.data);
            return Ok(Some(String::from_utf8_lossy(text).into_owned()));
        }
        // Medium strings: end offsets, one blob, and which are null.
        let offsets = self.node(leaf.child(0).ok_or_else(|| Error("no offsets".into()))?)?;
        let blob = self.node(leaf.child(1).ok_or_else(|| Error("no blob".into()))?)?;
        if let Some(nulls) = leaf.child(2).map(|at| self.node(at)).transpose()?
            && nulls.get(index) != 0
        {
            return Ok(None);
        }
        let begin = if index == 0 {
            0
        } else {
            offsets.get(index - 1) as usize
        };
        let end = offsets.get(index) as usize;
        let Some(text) = blob.data.get(begin..end) else {
            return Ok(None);
        };
        let text = text.strip_suffix(&[0]).unwrap_or(text);
        Ok(Some(String::from_utf8_lossy(text).into_owned()))
    }

    /// The object keys in a B+tree of them, as a list of links stores them.
    fn keys(&self, at: usize, out: &mut Vec<i64>) -> Result<()> {
        let node = self.node(at)?;
        if node.inner {
            // Offsets or a tagged child size first, the total size last.
            for index in 1..node.len.saturating_sub(1) {
                if let Some(child) = node.child(index) {
                    self.keys(child, out)?;
                }
            }
            return Ok(());
        }
        // A list stores the keys as they are; only a single link adds one.
        out.extend(node.values());
        Ok(())
    }
}

impl<'r, 'a> Table<'r, 'a> {
    pub fn column(&self, name: &str) -> Result<&Column> {
        self.columns
            .iter()
            .find(|column| column.name == name)
            .ok_or_else(|| Error(format!("{} has no column {name}", self.name)))
    }

    /// Every object, in key order within each cluster.
    pub fn objects(&self) -> Result<Vec<Obj<'a>>> {
        let mut out = Vec::new();
        if let Some(root) = self.root {
            self.walk(root, 0, &mut out)?;
        }
        Ok(out)
    }

    /// Every object by its key, for following links to this table.
    pub fn by_key(&self) -> Result<HashMap<i64, Obj<'a>>> {
        Ok(self
            .objects()?
            .into_iter()
            .map(|obj| (obj.key, obj))
            .collect())
    }

    fn walk(&self, at: usize, offset: i64, out: &mut Vec<Obj<'a>>) -> Result<()> {
        let node = self.realm.node(at)?;
        if node.inner {
            // The children's first keys (none when they are compact), the
            // depth, the number of objects below, then the children, whose
            // keys are relative to their entry here.
            let depth = node.tagged(1).unwrap_or(0);
            let shift = depth * 8;
            let keys = node.child(0).map(|at| self.realm.node(at)).transpose()?;
            for (slot, index) in (3..node.len).enumerate() {
                let Some(child) = node.child(index) else {
                    continue;
                };
                let base = match keys {
                    Some(keys) => keys.unsigned(slot),
                    None => (slot as i64) << shift,
                };
                self.walk(child, offset + base, out)?;
            }
            return Ok(());
        }
        let rows = match node.tagged(0) {
            Some(count) => (0..count).map(|key| offset + key).collect::<Vec<_>>(),
            None => {
                let keys = self.realm.node(
                    node.child(0)
                        .ok_or_else(|| Error("a cluster has no keys".into()))?,
                )?;
                (0..keys.len)
                    .map(|row| offset + keys.unsigned(row))
                    .collect()
            }
        };
        for (row, key) in rows.into_iter().enumerate() {
            out.push(Obj {
                key,
                cluster: node,
                row,
            });
        }
        Ok(())
    }

    fn leaf(&self, obj: &Obj<'a>, column: &Column) -> Result<Option<Node<'a>>> {
        obj.cluster
            .child(column.leaf)
            .map(|at| self.realm.node(at))
            .transpose()
    }

    pub fn string(&self, obj: &Obj<'a>, column: &Column) -> Result<Option<String>> {
        if column.kind != kind::STRING || column.is_collection() {
            return error(format!("{} is not a string", column.name));
        }
        match self.leaf(obj, column)? {
            Some(leaf) => self.realm.string_at(leaf, obj.row),
            None => Ok(None),
        }
    }

    /// An integer or a boolean; `None` when it is null.
    pub fn int(&self, obj: &Obj<'a>, column: &Column) -> Result<Option<i64>> {
        if !matches!(column.kind, kind::INT | kind::BOOL) || column.is_collection() {
            return error(format!("{} is not an integer", column.name));
        }
        let Some(leaf) = self.leaf(obj, column)? else {
            return Ok(None);
        };
        if column.nullable() {
            // The first element is the value that stands for null.
            let null = leaf.get(0);
            let value = leaf.get(obj.row + 1);
            return Ok((value != null).then_some(value));
        }
        Ok(Some(leaf.get(obj.row)))
    }

    pub fn double(&self, obj: &Obj<'a>, column: &Column) -> Result<Option<f64>> {
        if column.kind != kind::DOUBLE || column.is_collection() {
            return error(format!("{} is not a double", column.name));
        }
        let Some(leaf) = self.leaf(obj, column)? else {
            return Ok(None);
        };
        let Some(value) = leaf.float(obj.row) else {
            return Ok(None);
        };
        Ok((!value.is_nan()).then_some(value))
    }

    /// The key of the object a link points at.
    pub fn link(&self, obj: &Obj<'a>, column: &Column) -> Result<Option<i64>> {
        if column.kind != kind::LINK || column.is_collection() {
            return error(format!("{} is not a link", column.name));
        }
        let Some(leaf) = self.leaf(obj, column)? else {
            return Ok(None);
        };
        let stored = leaf.get(obj.row);
        Ok((stored != 0).then_some(stored - 1))
    }

    /// The keys of the objects a list of links points at, in order.
    pub fn links(&self, obj: &Obj<'a>, column: &Column) -> Result<Vec<i64>> {
        if column.kind != kind::LINK || !column.is_list() {
            return error(format!("{} is not a list of links", column.name));
        }
        let mut out = Vec::new();
        if let Some(leaf) = self.leaf(obj, column)?
            && let Some(list) = leaf.child(obj.row)
        {
            self.realm.keys(list, &mut out)?;
        }
        Ok(out)
    }
}

impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("inner", &self.inner)
            .field("has_refs", &self.has_refs)
            .field("context", &self.context)
            .field("width_type", &self.width_type)
            .field("width", &self.width)
            .field("len", &self.len)
            .field("file", &self.bytes.len())
            .finish()
    }
}

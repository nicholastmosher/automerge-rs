pub mod error;
mod serde_impls;
mod utility_impls;
use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    fmt,
    iter::FromIterator,
    num::NonZeroU32,
    slice::Iter,
};

use error::InvalidScalarValues;
use serde::{
    de::{Error, MapAccess, Unexpected},
    Deserialize, Serialize,
};
use smol_str::SmolStr;
use strum::EnumDiscriminants;
use tinyvec::TinyVec;

/// An actor id is a sequence of bytes. By default we use a uuid which can be nicely stack
/// allocated.
///
/// In the event that users want to use their own type of identifier that is longer than a uuid
/// then they will likely end up pushing it onto the heap which is still fine.
#[derive(Eq, PartialEq, Hash, Clone, PartialOrd, Ord)]
#[cfg_attr(feature = "derive-arbitrary", derive(arbitrary::Arbitrary))]
pub struct ActorId(TinyVec<[u8; 16]>);

impl fmt::Debug for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ActorID")
            .field(&hex::encode(&self.0))
            .finish()
    }
}

impl ActorId {
    pub fn random() -> ActorId {
        ActorId(TinyVec::from(*uuid::Uuid::new_v4().as_bytes()))
    }

    pub fn to_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_hex_string(&self) -> String {
        hex::encode(&self.0)
    }

    pub fn op_id_at(&self, seq: u64) -> OpId {
        OpId(seq, self.clone())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Copy, Hash)]
#[serde(rename_all = "camelCase", untagged)]
pub enum ObjType {
    Map,
    Table,
    List,
    Text,
}

impl ObjType {
    pub fn is_sequence(&self) -> bool {
        matches!(self, Self::List | Self::Text)
    }
}

impl From<MapType> for ObjType {
    fn from(other: MapType) -> Self {
        match other {
            MapType::Map => Self::Map,
            MapType::Table => Self::Table,
        }
    }
}

impl From<SequenceType> for ObjType {
    fn from(other: SequenceType) -> Self {
        match other {
            SequenceType::List => Self::List,
            SequenceType::Text => Self::Text,
        }
    }
}

impl fmt::Display for ObjType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjType::Map => write!(f, "map"),
            ObjType::Table => write!(f, "table"),
            ObjType::List => write!(f, "list"),
            ObjType::Text => write!(f, "text"),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Copy, Hash)]
#[cfg_attr(feature = "derive-arbitrary", derive(arbitrary::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub enum MapType {
    Map,
    Table,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Copy, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SequenceType {
    List,
    Text,
}

#[derive(Eq, PartialEq, Hash, Clone)]
#[cfg_attr(feature = "derive-arbitrary", derive(arbitrary::Arbitrary))]
pub struct OpId(pub u64, pub ActorId);

impl OpId {
    pub fn new(seq: u64, actor: &ActorId) -> OpId {
        OpId(seq, actor.clone())
    }

    pub fn counter(&self) -> u64 {
        self.0
    }

    pub fn increment_by(&self, by: u64) -> OpId {
        OpId(self.0 + by, self.1.clone())
    }

    /// Returns true if `other` has the same actor ID, and their counter is `delta` greater than
    /// ours.
    pub fn delta(&self, other: &Self, delta: u64) -> bool {
        self.1 == other.1 && self.0 + delta == other.0
    }
}

#[derive(Eq, PartialEq, Debug, Hash, Clone)]
#[cfg_attr(feature = "derive-arbitrary", derive(arbitrary::Arbitrary))]
pub enum ObjectId {
    Id(OpId),
    Root,
}

#[derive(PartialEq, Eq, Debug, Hash, Clone)]
pub enum ElementId {
    Head,
    Id(OpId),
}

impl ElementId {
    pub fn as_opid(&self) -> Option<&OpId> {
        match self {
            ElementId::Head => None,
            ElementId::Id(opid) => Some(opid),
        }
    }

    pub fn into_key(self) -> Key {
        Key::Seq(self)
    }

    pub fn not_head(&self) -> bool {
        match self {
            ElementId::Head => false,
            ElementId::Id(_) => true,
        }
    }

    pub fn increment_by(&self, by: u64) -> Option<Self> {
        match self {
            ElementId::Head => None,
            ElementId::Id(id) => Some(ElementId::Id(id.increment_by(by))),
        }
    }
}

#[derive(Serialize, PartialEq, Eq, Debug, Hash, Clone)]
#[serde(untagged)]
pub enum Key {
    Map(SmolStr),
    Seq(ElementId),
}

impl Key {
    pub fn head() -> Key {
        Key::Seq(ElementId::Head)
    }

    pub fn is_map_key(&self) -> bool {
        match self {
            Key::Map(_) => true,
            Key::Seq(_) => false,
        }
    }

    pub fn as_element_id(&self) -> Option<ElementId> {
        match self {
            Key::Map(_) => None,
            Key::Seq(eid) => Some(eid.clone()),
        }
    }

    pub fn to_opid(&self) -> Option<OpId> {
        match self.as_element_id()? {
            ElementId::Id(id) => Some(id),
            ElementId::Head => None,
        }
    }
    pub fn increment_by(&self, by: u64) -> Option<Self> {
        match self {
            Key::Map(_) => None,
            Key::Seq(eid) => eid.increment_by(by).map(Key::Seq),
        }
    }
}

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, Copy)]
pub enum DataType {
    #[serde(rename = "counter")]
    Counter,
    #[serde(rename = "timestamp")]
    Timestamp,
    #[serde(rename = "bytes")]
    Bytes,
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "uint")]
    Uint,
    #[serde(rename = "int")]
    Int,
    #[serde(rename = "float64")]
    F64,
    #[serde(rename = "undefined")]
    Undefined,
}

impl DataType {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn is_undefined(d: &DataType) -> bool {
        matches!(d, DataType::Undefined)
    }
}

/// We don't implement Serialize/Deserialize b/c
/// this struct will always be serialized as 2 fields
/// that are *part of* a larger struct. (It will never
/// be serialized as its own struct/map)
#[derive(PartialEq, Clone, Debug)]
pub struct ScalarValues {
    // For implementing Serialization in DiffEdit
    pub(crate) vec: Vec<ScalarValue>,
    // Can't use `std::mem::Discriminant` b/c we
    // need to be able to `match` on the kind for `as_numerical_datatype`...
    pub(crate) kind: ScalarValueKind,
}

impl ScalarValues {
    pub fn new(kind: ScalarValueKind) -> Self {
        Self {
            vec: Vec::new(),
            kind,
        }
    }

    pub fn from_values_and_datatype<'de, V: MapAccess<'de>>(
        mut old_values: Vec<ScalarValue>,
        datatype: Option<DataType>,
    ) -> Result<Self, V::Error> {
        // ensure the values can be cast to the correct datatype
        if let Some(datatype) = datatype {
            old_values = old_values
                .iter()
                .map(|v| {
                    v.as_datatype(datatype).map_err(|e| {
                        Error::invalid_value(
                            Unexpected::Other(e.unexpected.as_str()),
                            &e.expected.as_str(),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
        }
        old_values.try_into().map_err(|e| match e {
            InvalidScalarValues::Empty => Error::invalid_length(0, &"more than 0"),
            InvalidScalarValues::UnexpectedKind(exp, unexp) => {
                let unexp = format!("{:?}", unexp);
                let exp = format!("{:?}", exp);
                Error::invalid_value(Unexpected::Other(&unexp), &exp.as_str())
            }
        })
    }

    /// Try to append a `ScalarValue` to a `ScalarValues`. If we can't
    // returh the `ScalarValueKind` of the value we tried to add (for error reporting)
    pub fn append(&mut self, v: ScalarValue) -> Option<ScalarValueKind> {
        let new_kind = ScalarValueKind::from(&v);
        if self.kind == new_kind {
            self.vec.push(v);
            None
        } else {
            Some(new_kind)
        }
    }

    pub fn get(&self, idx: usize) -> Option<&ScalarValue> {
        self.vec.get(idx)
    }

    pub fn len(&self) -> usize {
        self.vec.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }

    pub fn iter(&self) -> Iter<ScalarValue> {
        self.vec.iter()
    }

    /// Returns an Option containing a `DataType` if
    /// `self` represents a numerical scalar value
    /// This is necessary b/c numerical values are not self-describing
    /// (unlike strings / bytes / etc. )
    pub fn as_numerical_datatype(&self) -> Option<DataType> {
        match self.kind {
            ScalarValueKind::Counter => Some(DataType::Counter),
            ScalarValueKind::Timestamp => Some(DataType::Timestamp),
            ScalarValueKind::Int => Some(DataType::Int),
            ScalarValueKind::Uint => Some(DataType::Uint),
            ScalarValueKind::F64 => Some(DataType::F64),
            _ => None,
        }
    }
}

#[derive(Serialize, PartialEq, Debug, Clone, EnumDiscriminants)]
#[strum_discriminants(name(ScalarValueKind))]
#[serde(untagged)]
pub enum ScalarValue {
    Bytes(Vec<u8>),
    Str(SmolStr),
    Int(i64),
    Uint(u64),
    F64(f64),
    Counter(i64),
    Timestamp(i64),
    Cursor(OpId),
    Boolean(bool),
    Null,
}

impl ScalarValue {
    pub fn as_datatype(
        &self,
        datatype: DataType,
    ) -> Result<ScalarValue, error::InvalidScalarValue> {
        match (datatype, self) {
            (DataType::Counter, ScalarValue::Int(i)) => Ok(ScalarValue::Counter(*i)),
            (DataType::Counter, ScalarValue::Uint(u)) => match i64::try_from(*u) {
                Ok(i) => Ok(ScalarValue::Counter(i)),
                Err(_) => Err(error::InvalidScalarValue {
                    raw_value: self.clone(),
                    expected: "an integer".to_string(),
                    unexpected: "an integer larger than i64::max_value".to_string(),
                    datatype,
                }),
            },
            (DataType::Bytes, ScalarValue::Bytes(bytes)) => Ok(ScalarValue::Bytes(bytes.clone())),
            (DataType::Bytes, v) => Err(error::InvalidScalarValue {
                raw_value: self.clone(),
                expected: "a vector of bytes".to_string(),
                unexpected: v.to_string(),
                datatype,
            }),
            (DataType::Counter, v) => Err(error::InvalidScalarValue {
                raw_value: self.clone(),
                expected: "an integer".to_string(),
                unexpected: v.to_string(),
                datatype,
            }),
            (DataType::Timestamp, ScalarValue::Int(i)) => Ok(ScalarValue::Timestamp(*i)),
            (DataType::Timestamp, ScalarValue::Uint(u)) => match i64::try_from(*u) {
                Ok(i) => Ok(ScalarValue::Timestamp(i)),
                Err(_) => Err(error::InvalidScalarValue {
                    raw_value: self.clone(),
                    expected: "an integer".to_string(),
                    unexpected: "an integer larger than i64::max_value".to_string(),
                    datatype,
                }),
            },
            (DataType::Timestamp, v) => Err(error::InvalidScalarValue {
                raw_value: self.clone(),
                expected: "an integer".to_string(),
                unexpected: v.to_string(),
                datatype,
            }),
            (DataType::Cursor, v) => Err(error::InvalidScalarValue {
                raw_value: self.clone(),
                expected: "a cursor".to_string(),
                unexpected: v.to_string(),
                datatype,
            }),
            (DataType::Int, v) => Ok(ScalarValue::Int(v.to_i64().ok_or(
                error::InvalidScalarValue {
                    raw_value: self.clone(),
                    expected: "an int".to_string(),
                    unexpected: v.to_string(),
                    datatype,
                },
            )?)),
            (DataType::Uint, v) => Ok(ScalarValue::Uint(v.to_u64().ok_or(
                error::InvalidScalarValue {
                    raw_value: self.clone(),
                    expected: "a uint".to_string(),
                    unexpected: v.to_string(),
                    datatype,
                },
            )?)),
            (DataType::F64, v) => Ok(ScalarValue::F64(v.to_f64().ok_or(
                error::InvalidScalarValue {
                    raw_value: self.clone(),
                    expected: "an f64".to_string(),
                    unexpected: v.to_string(),
                    datatype,
                },
            )?)),
            (DataType::Undefined, _) => Ok(self.clone()),
        }
    }

    /// Returns an Option containing a `DataType` if
    /// `self` represents a numerical scalar value
    /// This is necessary b/c numerical values are not self-describing
    /// (unlike strings / bytes / etc. )
    pub fn as_numerical_datatype(&self) -> Option<DataType> {
        match self {
            ScalarValue::Counter(..) => Some(DataType::Counter),
            ScalarValue::Timestamp(..) => Some(DataType::Timestamp),
            ScalarValue::Int(..) => Some(DataType::Int),
            ScalarValue::Uint(..) => Some(DataType::Uint),
            ScalarValue::F64(..) => Some(DataType::F64),
            _ => None,
        }
    }

    // TODO: Should this method be combined with as_numerical_datatype??
    pub fn datatype(&self) -> Option<DataType> {
        match self {
            ScalarValue::Counter(..) => Some(DataType::Counter),
            ScalarValue::Timestamp(..) => Some(DataType::Timestamp),
            ScalarValue::Int(..) => Some(DataType::Int),
            ScalarValue::Uint(..) => Some(DataType::Uint),
            ScalarValue::F64(..) => Some(DataType::F64),
            ScalarValue::Cursor(..) => Some(DataType::Cursor),
            _ => None,
        }
    }

    /// If this value can be coerced to an i64, return the i64 value
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            ScalarValue::Int(n) => Some(*n),
            ScalarValue::Uint(n) => Some(*n as i64),
            ScalarValue::F64(n) => Some(*n as i64),
            ScalarValue::Counter(n) => Some(*n),
            ScalarValue::Timestamp(n) => Some(*n),
            _ => None,
        }
    }

    pub fn to_u64(&self) -> Option<u64> {
        match self {
            ScalarValue::Int(n) => Some(*n as u64),
            ScalarValue::Uint(n) => Some(*n),
            ScalarValue::F64(n) => Some(*n as u64),
            ScalarValue::Counter(n) => Some(*n as u64),
            ScalarValue::Timestamp(n) => Some(*n as u64),
            _ => None,
        }
    }

    pub fn to_f64(&self) -> Option<f64> {
        match self {
            ScalarValue::Int(n) => Some(*n as f64),
            ScalarValue::Uint(n) => Some(*n as f64),
            ScalarValue::F64(n) => Some(*n),
            ScalarValue::Counter(n) => Some(*n as f64),
            ScalarValue::Timestamp(n) => Some(*n as f64),
            _ => None,
        }
    }
}

#[derive(PartialEq, Debug, Clone)]
pub enum OpType {
    Make(ObjType),
    /// Perform a deletion, expanding the operation to cover `n` deletions (multiOp).
    Del(NonZeroU32),
    Inc(i64),
    Set(ScalarValue),
    MultiSet(ScalarValues),
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SortedVec<T>(Vec<T>);

impl<T> SortedVec<T> {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.0.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.0.get_mut(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.0.iter()
    }
}

impl<T: Ord> From<Vec<T>> for SortedVec<T> {
    fn from(mut other: Vec<T>) -> Self {
        other.sort_unstable();
        Self(other)
    }
}

impl<T: Ord> FromIterator<T> for SortedVec<T> {
    fn from_iter<I>(iter: I) -> Self
    where
        I: std::iter::IntoIterator<Item = T>,
    {
        let mut inner: Vec<T> = iter.into_iter().collect();
        inner.sort_unstable();
        Self(inner)
    }
}

impl<T> IntoIterator for SortedVec<T> {
    type Item = T;

    type IntoIter = <Vec<T> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'de, T> serde::Deserialize<'de> for SortedVec<T>
where
    T: serde::Deserialize<'de> + Ord,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut v = Vec::deserialize(deserializer)?;
        v.sort_unstable();
        Ok(Self(v))
    }
}

#[derive(PartialEq, Debug, Clone)]
pub struct Op {
    pub action: OpType,
    pub obj: ObjectId,
    pub key: Key,
    pub pred: SortedVec<OpId>,
    pub insert: bool,
}

impl Op {
    pub fn primitive_value(&self) -> Option<ScalarValue> {
        match &self.action {
            OpType::Set(v) => Some(v.clone()),
            OpType::Inc(i) => Some(ScalarValue::Int(*i)),
            _ => None,
        }
    }

    pub fn obj_type(&self) -> Option<ObjType> {
        match self.action {
            OpType::Make(o) => Some(o),
            _ => None,
        }
    }

    pub fn to_i64(&self) -> Option<i64> {
        self.primitive_value().as_ref().and_then(|v| v.to_i64())
    }
}

#[derive(Eq, PartialEq, Hash, Clone, PartialOrd, Ord, Copy)]
pub struct ChangeHash(pub [u8; 32]);

impl fmt::Debug for ChangeHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ChangeHash")
            .field(&hex::encode(&self.0))
            .finish()
    }
}

// The Diff Structure Maps on to the Patch Diffs the Frontend is expecting
// Diff {
//  object_id: 123,
//  obj_type: map,
//  props: {
//      "key1": {
//          "10@abc123":
//              DiffLink::Diff(Diff {
//                  object_id: 444,
//                  obj_type: list,
//                  edits: [ DiffEdit { ... } ],
//                  props: { ... },
//              })
//          }
//      "key2": {
//          "11@abc123":
//              DiffLink::Value(DiffValue {
//                  value: 10,
//                  datatype: "counter"
//              }
//          }
//      }
// }

#[derive(Debug, PartialEq, Clone)]
pub enum Diff {
    Map(MapDiff),
    Table(TableDiff),
    List(ListDiff),
    Text(TextDiff),
    Value(ScalarValue),
    Cursor(CursorDiff),
}

impl Diff {
    pub fn object_type(&self) -> Option<ObjType> {
        match self {
            Diff::Map(_) => Some(ObjType::Map),
            Diff::Table(_) => Some(ObjType::Table),
            Diff::List(_) => Some(ObjType::List),
            Diff::Text(_) => Some(ObjType::Text),
            Diff::Value(_) => None,
            Diff::Cursor(_) => None,
        }
    }

    pub fn object_id(&self) -> Option<ObjectId> {
        match self {
            Diff::Map(mapdiff) => Some(mapdiff.object_id.clone()),
            Diff::Table(tablediff) => Some(tablediff.object_id.clone()),
            Diff::List(listdiff) => Some(listdiff.object_id.clone()),
            Diff::Text(textdiff) => Some(textdiff.object_id.clone()),
            Diff::Value(..) => None,
            Diff::Cursor(CursorDiff { object_id, .. }) => Some(object_id.clone()),
        }
    }
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MapDiff {
    pub object_id: ObjectId,
    pub props: HashMap<SmolStr, HashMap<OpId, Diff>>,
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TableDiff {
    pub object_id: ObjectId,
    pub props: HashMap<SmolStr, HashMap<OpId, Diff>>,
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ListDiff {
    pub object_id: ObjectId,
    pub edits: Vec<DiffEdit>,
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TextDiff {
    pub object_id: ObjectId,
    pub edits: Vec<DiffEdit>,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ObjDiff {
    pub object_id: ObjectId,
    #[serde(rename = "type")]
    pub obj_type: ObjType,
}

#[derive(Debug, PartialEq, Clone)]
pub struct CursorDiff {
    pub object_id: ObjectId,
    pub elem_id: OpId,
    pub index: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase", tag = "action")]
pub enum DiffEdit {
    /// Describes the insertion of a single element into a list or text object.
    /// The element can be a nested object.
    #[serde(rename = "insert", rename_all = "camelCase")]
    SingleElementInsert {
        /// the list index at which to insert the new element
        index: u64,
        /// the unique element ID of the new list element
        elem_id: ElementId,
        /// ID of the operation that assigned this value
        op_id: OpId,
        value: Diff,
    },
    /// Describes the insertion of a consecutive sequence of primitive values into
    /// a list or text object. In the case of text, the values are strings (each
    /// character as a separate string value). Each inserted value is given a
    /// consecutive element ID: starting with `elemId` for the first value, the
    /// subsequent values are given elemIds with the same actor ID and incrementing
    /// counters. To insert non-primitive values, use SingleInsertEdit.

    /// We need to use a separate struct here to implement custom
    /// serialization and deserialization logic (due to the presence
    /// of the datatype field)
    #[serde(rename = "multi-insert")]
    MultiElementInsert(MultiElementInsert),

    /// Describes the update of the value or nested object at a particular index
    /// of a list or text object. In the case where there are multiple conflicted
    /// values at the same list index, multiple UpdateEdits with the same index
    /// (but different opIds) appear in the edits array of ListDiff.
    #[serde(rename_all = "camelCase")]
    Update {
        /// the list index to update
        index: u64,
        /// ID of the operation that assigned this value
        op_id: OpId,
        value: Diff,
    },
    #[serde(rename_all = "camelCase")]
    Remove { index: u64, count: u64 },
}

#[derive(Debug, PartialEq, Clone)]
pub struct MultiElementInsert {
    /// the list index at which to insert the first value
    pub index: u64,
    /// the unique ID of the first inserted element
    pub elem_id: ElementId,
    pub values: ScalarValues,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub actor: Option<ActorId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub seq: Option<u64>,
    pub clock: HashMap<ActorId, u64>,
    pub deps: Vec<ChangeHash>,
    pub max_op: u64,
    pub pending_changes: usize,
    //    pub can_undo: bool,
    //    pub can_redo: bool,
    //    pub version: u64,
    pub diffs: RootDiff,
}

/// A custom MapDiff that implicitly has the object_id Root and is a map object.
#[derive(Debug, PartialEq, Clone, Default)]
pub struct RootDiff {
    pub props: HashMap<SmolStr, HashMap<OpId, Diff>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Change {
    #[serde(rename = "ops")]
    pub operations: Vec<Op>,
    #[serde(rename = "actor")]
    pub actor_id: ActorId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hash: Option<ChangeHash>,
    pub seq: u64,
    #[serde(rename = "startOp")]
    pub start_op: u64,
    pub time: i64,
    pub message: Option<String>,
    pub deps: Vec<ChangeHash>,
    #[serde(skip_serializing_if = "Vec::is_empty", default = "Default::default")]
    pub extra_bytes: Vec<u8>,
}

impl PartialEq for Change {
    // everything but hash (its computed and not always present)
    fn eq(&self, other: &Self) -> bool {
        self.operations == other.operations
            && self.actor_id == other.actor_id
            && self.seq == other.seq
            && self.start_op == other.start_op
            && self.time == other.time
            && self.message == other.message
            && self.deps == other.deps
            && self.extra_bytes == other.extra_bytes
    }
}

impl Change {
    pub fn op_id_of(&self, index: u64) -> Option<OpId> {
        if let Ok(index_usize) = usize::try_from(index) {
            if index_usize < self.operations.len() {
                return Some(self.actor_id.op_id_at(self.start_op + index));
            }
        }
        None
    }
}

//! Shared test support for the T1a pack (VFL-112, covers C1 / VFL-14).
//!
//! `vf-core`'s `Cargo.toml` dev-dependencies are scoped to what
//! `crates/vf-core/tests/scope_match.rs` (pack T2) needs (`proptest`) and do
//! not include `serde_json`; the lane gate (`ci/lane-gate.sh`) refuses a
//! pull request that touches `Cargo.toml` and `tests/` at once (ADR-0005),
//! so this test-only pull request cannot add it to inspect what
//! `serde::Serialize` produces.
//!
//! This module is the workaround: a minimal `Value` tree plus a
//! `serde::Serializer`/`serde::Deserializer` pair that builds and reads it,
//! using only `serde` itself — a dependency `vf-core` already has. It is
//! intentionally not a general-purpose format: methods the catalogue under
//! test never calls (bytes, floats, tuple/struct variants) fail loudly rather
//! than guess, so a future variant that needs one is a visible test failure
//! here, not a silent gap.
//!
//! Each of `ids.rs`, `problem.rs` and `state_enums.rs` pulls this module in
//! via `mod support;` and compiles it as part of its own test binary, using
//! only the subset of `Value`/serializer/deserializer items that pack needs.
//! `#![allow(dead_code)]` keeps `cargo clippy --all-targets -- -D warnings`
//! from flagging the items a given binary doesn't touch as unused; `#[expect]`
//! can't be used in its place because some other binary does use them.
#![allow(dead_code)]

use std::fmt;

use serde::de::{DeserializeOwned, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, SerializeStruct};
use serde::{Deserializer, Serialize, Serializer};

/// A minimal, order-preserving value tree — enough to represent anything
/// `vf-core`'s hand-written and derived `Serialize` impls produce.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Bool(bool),
    U64(u64),
    None,
    Seq(Vec<Value>),
    /// Field/entry order as emitted, not sorted — tests that assert on key
    /// order (the RFC 9457 member order, for instance) read this directly.
    Map(Vec<(String, Value)>),
}

impl Value {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U64(n) => Some(*n),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    #[must_use]
    pub fn as_seq(&self) -> Option<&[Value]> {
        match self {
            Self::Seq(items) => Some(items),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_map(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Map(entries) => Some(entries),
            _ => None,
        }
    }

    /// The value of the first entry named `key`, if this is a map and the key
    /// is present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_map()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Whether this is a map containing exactly `key`, in that order.
    #[must_use]
    pub fn has_keys_in_order(&self, keys: &[&str]) -> bool {
        match self.as_map() {
            Some(entries) => entries.iter().map(|(k, _)| k.as_str()).eq(keys.iter().copied()),
            None => false,
        }
    }
}

/// The error type for both directions. The test Value tree is always built
/// from a successful `Serialize` impl and always read into a type whose shape
/// matches, so every variant below indicates a bug in this support module or
/// an unanticipated production shape — not an expected test outcome.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl serde::ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

fn unsupported(what: &str) -> Error {
    Error(format!("test Value tree does not support {what}"))
}

// ---------------------------------------------------------------------------
// Serializer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct ValueSerializer;

pub struct SeqSerializer {
    items: Vec<Value>,
}

pub struct MapSerializer {
    entries: Vec<(String, Value)>,
    pending_key: Option<String>,
}

impl MapSerializer {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            pending_key: None,
        }
    }
}

impl Serializer for ValueSerializer {
    type Ok = Value;
    type Error = Error;
    type SerializeSeq = SeqSerializer;
    type SerializeTuple = SeqSerializer;
    type SerializeTupleStruct = SeqSerializer;
    type SerializeTupleVariant = SeqSerializer;
    type SerializeMap = MapSerializer;
    type SerializeStruct = MapSerializer;
    type SerializeStructVariant = MapSerializer;

    fn serialize_bool(self, v: bool) -> Result<Value, Error> {
        Ok(Value::Bool(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Value, Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i16(self, v: i16) -> Result<Value, Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i32(self, v: i32) -> Result<Value, Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i64(self, v: i64) -> Result<Value, Error> {
        u64::try_from(v)
            .map(Value::U64)
            .map_err(|_| unsupported("negative integers"))
    }

    fn serialize_u8(self, v: u8) -> Result<Value, Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u16(self, v: u16) -> Result<Value, Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u32(self, v: u32) -> Result<Value, Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u64(self, v: u64) -> Result<Value, Error> {
        Ok(Value::U64(v))
    }

    fn serialize_f32(self, _v: f32) -> Result<Value, Error> {
        Err(unsupported("f32"))
    }
    fn serialize_f64(self, _v: f64) -> Result<Value, Error> {
        Err(unsupported("f64"))
    }

    fn serialize_char(self, v: char) -> Result<Value, Error> {
        Ok(Value::Str(v.to_string()))
    }

    fn serialize_str(self, v: &str) -> Result<Value, Error> {
        Ok(Value::Str(v.to_owned()))
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Value, Error> {
        Err(unsupported("byte strings"))
    }

    fn serialize_none(self) -> Result<Value, Error> {
        Ok(Value::None)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Value, Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value, Error> {
        Ok(Value::None)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value, Error> {
        Ok(Value::None)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Value, Error> {
        Ok(Value::Str(variant.to_owned()))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Value, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value, Error> {
        Ok(Value::Map(vec![(variant.to_owned(), value.serialize(self)?)]))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<SeqSerializer, Error> {
        Ok(SeqSerializer {
            items: Vec::with_capacity(len.unwrap_or(0)),
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<SeqSerializer, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<SeqSerializer, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<SeqSerializer, Error> {
        Err(unsupported("tuple variants"))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<MapSerializer, Error> {
        Ok(MapSerializer::new())
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<MapSerializer, Error> {
        Ok(MapSerializer::new())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<MapSerializer, Error> {
        Err(unsupported("struct variants"))
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

impl SerializeSeq for SeqSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        self.items.push(value.serialize(ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Value, Error> {
        Ok(Value::Seq(self.items))
    }
}

impl serde::ser::SerializeTuple for SeqSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, Error> {
        SerializeSeq::end(self)
    }
}

impl serde::ser::SerializeTupleStruct for SeqSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, Error> {
        SerializeSeq::end(self)
    }
}

impl serde::ser::SerializeTupleVariant for SeqSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, Error> {
        SerializeSeq::end(self)
    }
}

impl SerializeMap for MapSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Error> {
        let key = match key.serialize(ValueSerializer)? {
            Value::Str(s) => s,
            other => return Err(Error(format!("map key is not a string: {other:?}"))),
        };
        self.pending_key = Some(key);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        let key = self
            .pending_key
            .take()
            .ok_or_else(|| Error("serialize_value called before serialize_key".into()))?;
        self.entries.push((key, value.serialize(ValueSerializer)?));
        Ok(())
    }

    fn end(self) -> Result<Value, Error> {
        Ok(Value::Map(self.entries))
    }
}

impl SerializeStruct for MapSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.entries
            .push((key.to_owned(), value.serialize(ValueSerializer)?));
        Ok(())
    }

    fn skip_field(&mut self, _key: &'static str) -> Result<(), Error> {
        Ok(())
    }

    fn end(self) -> Result<Value, Error> {
        Ok(Value::Map(self.entries))
    }
}

impl serde::ser::SerializeStructVariant for MapSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Result<Value, Error> {
        SerializeStruct::end(self)
    }
}

/// Serializes `value` into the test [`Value`] tree.
#[must_use]
pub fn to_value<T: Serialize + ?Sized>(value: &T) -> Value {
    value
        .serialize(ValueSerializer)
        .expect("the test Value serializer never fails on vf-core's Serialize impls")
}

// ---------------------------------------------------------------------------
// Deserializer
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct ValueDeserializer<'a> {
    value: &'a Value,
}

struct SeqDeserializer<'a> {
    items: std::slice::Iter<'a, Value>,
}

impl<'de> SeqAccess<'de> for SeqDeserializer<'de> {
    type Error = Error;

    fn next_element_seed<T: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        match self.items.next() {
            Some(item) => seed.deserialize(ValueDeserializer { value: item }).map(Some),
            None => Ok(None),
        }
    }
}

struct MapDeserializer<'a> {
    entries: std::slice::Iter<'a, (String, Value)>,
    value: Option<&'a Value>,
}

impl<'de> MapAccess<'de> for MapDeserializer<'de> {
    type Error = Error;

    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        match self.entries.next() {
            Some((key, value)) => {
                self.value = Some(value);
                seed.deserialize(key.as_str().into_deserializer()).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Error> {
        let value = self
            .value
            .take()
            .ok_or_else(|| Error("next_value_seed called before next_key_seed".into()))?;
        seed.deserialize(ValueDeserializer { value })
    }
}

impl<'de> Deserializer<'de> for ValueDeserializer<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.value {
            Value::Str(s) => visitor.visit_borrowed_str(s.as_str()),
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::U64(n) => visitor.visit_u64(*n),
            Value::None => visitor.visit_none(),
            Value::Seq(items) => visitor.visit_seq(SeqDeserializer { items: items.iter() }),
            Value::Map(entries) => visitor.visit_map(MapDeserializer {
                entries: entries.iter(),
                value: None,
            }),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.value {
            Value::None => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn is_human_readable(&self) -> bool {
        true
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct newtype_struct seq tuple tuple_struct
        map struct enum identifier ignored_any
    }
}

/// Deserializes a value of type `T` out of the test [`Value`] tree.
///
/// Combined with [`to_value`], this is the round trip the `ids` and
/// `state_enums` acceptance criteria name — serialize, then deserialize the
/// result back and compare — without pulling in `serde_json` (see the module
/// docs).
#[must_use]
pub fn from_value<T: DeserializeOwned>(value: &Value) -> T {
    T::deserialize(ValueDeserializer { value })
        .expect("the test Value deserializer never fails on a Value it built itself")
}

/// Polls `fut` once and returns its output.
///
/// `vf-core` carries no async runtime (architecture §A1.3), so the F3a
/// port-contract tests (VFL-314 / VFL-355, T4 slices of VFL-44 for F3a /
/// VFL-310) drive the `PortFuture` values the ports return by hand instead
/// of reaching for `tokio`. Every future under test in that pack either is,
/// or is built from, `std::future::ready` (the same primitive `vf-core`'s
/// own `TenantArtifactStore::refuse` helper uses, and the only kind of
/// future a hand-written test double in that pack produces), so it is
/// always `Ready` on the first poll. A single poll with `Waker::noop()` is
/// therefore enough; a `Pending` result means a test double is not what
/// that pack assumes it is, not that the caller should wait and retry.
///
/// # Panics
///
/// If `fut` is not `Ready` on the first poll.
pub fn block_on<T>(
    mut fut: std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + '_>>,
) -> T {
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(value) => value,
        std::task::Poll::Pending => panic!(
            "block_on: future was not Ready on the first poll; this helper \
             only drives futures that resolve synchronously, which is \
             everything under test in this pack"
        ),
    }
}

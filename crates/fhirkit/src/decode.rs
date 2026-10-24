//! Reading and writing FHIR JSON for generated types.
//!
//! Decoding never stops at the first problem. Every problem becomes an
//! [`Issue`] with the FHIRPath-style location of the element
//! (`Observation.component[1].valueQuantity.value`), and decoding carries
//! on with what it could read, the way a validator reports rather than a
//! parser that gives up. Callers decide what an issue costs them: see
//! [`crate::from_json`] and [`crate::from_json_strict`].

use crate::json::Json;
use crate::primitive::{Kind, Primitive, PrimitiveValue};
use crate::r4::Extension;
use std::fmt;

/// How bad a problem is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The content is not valid FHIR; the element was dropped or is wrong.
    Error,
    /// Valid, but questionable (an element a later reader may not expect).
    Warning,
}

/// One problem found while reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// How bad it is.
    pub severity: Severity,
    /// Where: `Patient.name[0].given[1]`.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{s} at {}: {}", self.path, self.message)
    }
}

/// Collects issues and tracks the current location while decoding.
#[derive(Debug, Default)]
pub struct Decoder {
    path: Vec<String>,
    /// Everything found so far.
    pub issues: Vec<Issue>,
}

impl Decoder {
    /// A decoder whose paths start at `root` (a resource or type name).
    pub fn new(root: &str) -> Decoder {
        Decoder {
            path: vec![root.to_string()],
            issues: Vec::new(),
        }
    }

    /// The current location.
    pub fn path(&self) -> String {
        let mut s = String::new();
        for (i, p) in self.path.iter().enumerate() {
            if i > 0 && !p.starts_with('[') {
                s.push('.');
            }
            s.push_str(p);
        }
        s
    }

    /// Report an error at the current location.
    pub fn error(&mut self, message: impl Into<String>) {
        self.issues.push(Issue {
            severity: Severity::Error,
            path: self.path(),
            message: message.into(),
        });
    }

    /// Report a warning at the current location.
    pub fn warning(&mut self, message: impl Into<String>) {
        self.issues.push(Issue {
            severity: Severity::Warning,
            path: self.path(),
            message: message.into(),
        });
    }

    /// Run `f` one level deeper, at `segment`.
    pub fn at<R>(&mut self, segment: impl Into<String>, f: impl FnOnce(&mut Decoder) -> R) -> R {
        self.path.push(segment.into());
        let r = f(self);
        self.path.pop();
        r
    }

    /// Replace the last path segment's root (for `contained` and bundle
    /// entries, where the resource type is only known after reading it).
    pub fn root(&self) -> &str {
        self.path.first().map_or("", String::as_str)
    }
}

/// A FHIR type that reads and writes its JSON form. Generated for every
/// complex type, backbone element and resource.
pub trait FhirJson: Sized {
    /// The FHIR type name, used as the path root when decoding on its own.
    const TYPE_NAME: &'static str;

    /// Read from JSON, reporting problems to `d`. `None` when the value
    /// was not usable at all (not an object, say).
    fn decode(v: &Json, d: &mut Decoder) -> Option<Self>;

    /// Write the JSON form.
    fn encode(&self) -> Json;
}

/// A `[x]` choice element: an enum with one variant per allowed type.
pub trait Choice: Sized {
    /// The FHIR type codes the element allows, in definition order. The
    /// JSON key is the element name followed by the capitalised code
    /// (`value` + `Quantity`).
    const TYPES: &'static [&'static str];

    /// Read the variant for type `code` from key `key` of `o`.
    fn decode_as(code: &str, key: &str, o: &mut Obj<'_>, d: &mut Decoder) -> Option<Self>;

    /// Write the chosen variant under `base` plus its type suffix.
    fn encode_into(&self, base: &str, w: &mut ObjWriter);
}

/// The JSON key for choice element `base` holding type `code`.
pub fn choice_key(base: &str, code: &str) -> String {
    let mut k = String::with_capacity(base.len() + code.len());
    k.push_str(base);
    let mut c = code.chars();
    if let Some(first) = c.next() {
        k.extend(first.to_uppercase());
        k.push_str(c.as_str());
    }
    k
}

/// An object being read: tracks which members were used, so the rest can
/// be reported as unknown.
pub struct Obj<'a> {
    members: &'a [(String, Json)],
    used: Vec<bool>,
}

impl<'a> Obj<'a> {
    /// Start reading `v`, which must be an object.
    pub fn new(v: &'a Json, d: &mut Decoder) -> Option<Obj<'a>> {
        match v {
            Json::Object(m) => {
                let mut seen: Vec<&str> = Vec::new();
                for (k, _) in m {
                    if seen.contains(&k.as_str()) {
                        d.at(k.clone(), |d| d.error("the key appears more than once"));
                    } else {
                        seen.push(k);
                    }
                }
                Some(Obj {
                    members: m,
                    used: vec![false; m.len()],
                })
            }
            other => {
                d.error(format!("expected an object, found {}", other.kind()));
                None
            }
        }
    }

    /// Take member `key`, marking it used.
    pub fn take(&mut self, key: &str) -> Option<&'a Json> {
        let i = self.members.iter().position(|(k, _)| k == key)?;
        self.used[i] = true;
        Some(&self.members[i].1)
    }

    /// `true` when member `key` exists, without using it.
    pub fn has(&self, key: &str) -> bool {
        self.members.iter().any(|(k, _)| k == key)
    }

    /// A plain string member (element and resource `id`).
    pub fn string(&mut self, d: &mut Decoder, key: &str) -> Option<String> {
        let v = self.take(key)?;
        d.at(key, |d| match v {
            Json::String(s) if !s.is_empty() => Some(s.clone()),
            Json::String(_) => {
                d.error("an empty string is not allowed");
                None
            }
            other => {
                d.error(format!("expected a string, found {}", other.kind()));
                None
            }
        })
    }

    /// A single complex value.
    pub fn one<T: FhirJson>(&mut self, d: &mut Decoder, key: &str) -> Option<T> {
        let v = self.take(key)?;
        d.at(key, |d| {
            if matches!(v, Json::Array(_)) {
                d.error("expected a single value, found an array");
                return None;
            }
            T::decode(v, d)
        })
    }

    /// A repeating complex value.
    pub fn list<T: FhirJson>(&mut self, d: &mut Decoder, key: &str) -> Vec<T> {
        let Some(v) = self.take(key) else {
            return Vec::new();
        };
        let Json::Array(items) = v else {
            d.at(key, |d| {
                d.error(format!("expected an array, found {}", v.kind()))
            });
            return Vec::new();
        };
        if items.is_empty() {
            d.at(key, |d| {
                d.error("an empty array is not allowed; leave the element out")
            });
        }
        items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                d.at(key, |d| {
                    d.at(format!("[{i}]"), |d| {
                        if matches!(item, Json::Null) {
                            d.error("null is not allowed in an array of complex values");
                            None
                        } else {
                            T::decode(item, d)
                        }
                    })
                })
            })
            .collect()
    }

    /// A single primitive, with its `_key` twin.
    pub fn prim<T: PrimitiveValue>(
        &mut self,
        d: &mut Decoder,
        key: &str,
        kind: Kind,
    ) -> Option<Primitive<T>> {
        let value = self.take(key);
        let twin_key = format!("_{key}");
        let twin = self.take(&twin_key);
        if value.is_none() && twin.is_none() {
            return None;
        }
        let mut p = Primitive::default();
        if let Some(v) = value {
            d.at(key, |d| match v {
                Json::Array(_) => d.error("expected a single value, found an array"),
                Json::Null => d.error("null is not allowed; leave the element out"),
                v => match T::from_json(v, kind) {
                    Ok(x) => p.value = Some(x),
                    Err(e) => d.error(e),
                },
            });
        }
        if let Some(t) = twin {
            d.at(twin_key, |d| element_parts(t, d, &mut p));
        }
        Some(p)
    }

    /// A repeating primitive: `key` and `_key` arrays aligned by index,
    /// `null` filling the gaps.
    pub fn prim_list<T: PrimitiveValue>(
        &mut self,
        d: &mut Decoder,
        key: &str,
        kind: Kind,
    ) -> Vec<Primitive<T>> {
        let values = self.take(key);
        let twin_key = format!("_{key}");
        let twins = self.take(&twin_key);
        let as_array = |v: Option<&'a Json>, k: &str, d: &mut Decoder| -> Option<&'a [Json]> {
            match v? {
                Json::Array(a) => Some(a.as_slice()),
                other => {
                    d.at(k.to_string(), |d| {
                        d.error(format!("expected an array, found {}", other.kind()))
                    });
                    None
                }
            }
        };
        let values = as_array(values, key, d).unwrap_or(&[]);
        let twins = as_array(twins, &twin_key, d).unwrap_or(&[]);
        if !twins.is_empty() && !values.is_empty() && twins.len() != values.len() {
            d.at(twin_key.clone(), |d| {
                d.error(format!(
                    "has {} entries but {key} has {}; the two arrays must line up",
                    twins.len(),
                    values.len()
                ))
            });
        }
        let n = values.len().max(twins.len());
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let mut p = Primitive::default();
            if let Some(v) = values.get(i) {
                d.at(key, |d| {
                    d.at(format!("[{i}]"), |d| {
                        if !matches!(v, Json::Null) {
                            match T::from_json(v, kind) {
                                Ok(x) => p.value = Some(x),
                                Err(e) => d.error(e),
                            }
                        }
                    })
                });
            }
            if let Some(t) = twins.get(i).filter(|t| !matches!(t, Json::Null)) {
                d.at(twin_key.clone(), |d| {
                    d.at(format!("[{i}]"), |d| element_parts(t, d, &mut p))
                });
            }
            if p.is_empty() {
                d.at(key, |d| {
                    d.at(format!("[{i}]"), |d| {
                        d.error("null with no extension in its place")
                    })
                });
            }
            out.push(p);
        }
        out
    }

    /// A `[x]` choice element: at most one of its typed keys may appear.
    pub fn choice<C: Choice>(&mut self, d: &mut Decoder, base: &str) -> Option<C> {
        let present: Vec<(&str, String)> = C::TYPES
            .iter()
            .map(|code| (*code, choice_key(base, code)))
            .filter(|(_, k)| self.has(k) || self.has(&format!("_{k}")))
            .collect();
        if present.len() > 1 {
            let keys: Vec<&str> = present.iter().map(|(_, k)| k.as_str()).collect();
            d.at(format!("{base}[x]"), |d| {
                d.error(format!(
                    "only one type may be given, found {}",
                    keys.join(", ")
                ))
            });
        }
        let (code, key) = present.into_iter().next()?;
        C::decode_as(code, &key, self, d)
    }

    /// Report the members nobody used.
    pub fn finish(self, d: &mut Decoder) {
        for ((k, _), used) in self.members.iter().zip(self.used) {
            if !used && k != "fhir_comments" {
                d.at(k.clone(), |d| d.error("unknown element"));
            }
        }
    }
}

/// Read the `{"id": .., "extension": [..]}` twin of a primitive.
fn element_parts<T>(t: &Json, d: &mut Decoder, p: &mut Primitive<T>) {
    let Some(mut o) = Obj::new(t, d) else { return };
    p.id = o.string(d, "id");
    p.extension = o.list::<Extension>(d, "extension");
    o.finish(d);
}

/// Report a missing required element. Call after decoding a field whose
/// minimum cardinality is 1.
pub fn required<T>(d: &mut Decoder, key: &str, v: &Option<T>) {
    if v.is_none() {
        d.at(key, |d| d.error("required element missing"));
    }
}

/// Report a repeating element with fewer than `min` entries.
pub fn required_list<T>(d: &mut Decoder, key: &str, v: &[T], min: usize) {
    if v.len() < min {
        d.at(key, |d| {
            d.error(if min == 1 {
                "required element missing".to_string()
            } else {
                format!("needs at least {min} entries, found {}", v.len())
            })
        });
    }
}

/// An object being written, members in the order they are added.
#[derive(Default)]
pub struct ObjWriter {
    members: Vec<(String, Json)>,
}

impl ObjWriter {
    /// An empty object.
    pub fn new() -> ObjWriter {
        ObjWriter::default()
    }

    /// Add a raw member.
    pub fn raw(&mut self, key: &str, v: Json) {
        self.members.push((key.to_string(), v));
    }

    /// A plain string member, if set.
    pub fn string(&mut self, key: &str, v: &Option<String>) {
        if let Some(s) = v {
            self.raw(key, Json::String(s.clone()));
        }
    }

    /// A single complex value, if set.
    pub fn one<T: FhirJson>(&mut self, key: &str, v: &Option<T>) {
        if let Some(x) = v {
            self.raw(key, x.encode());
        }
    }

    /// A complex value that is always there (a choice variant's payload).
    pub fn value<T: FhirJson>(&mut self, key: &str, v: &T) {
        self.raw(key, v.encode());
    }

    /// A repeating complex value, if not empty.
    pub fn list<T: FhirJson>(&mut self, key: &str, v: &[T]) {
        if !v.is_empty() {
            self.raw(key, Json::Array(v.iter().map(FhirJson::encode).collect()));
        }
    }

    /// A single primitive and its `_key` twin.
    pub fn prim<T: PrimitiveValue>(&mut self, key: &str, v: &Option<Primitive<T>>) {
        if let Some(p) = v {
            self.prim_value(key, p);
        }
    }

    /// A primitive that is always there (a choice variant's payload).
    pub fn prim_value<T: PrimitiveValue>(&mut self, key: &str, p: &Primitive<T>) {
        if let Some(x) = &p.value {
            self.raw(key, x.to_json());
        }
        if let Some(parts) = parts_json(p) {
            self.raw(&format!("_{key}"), parts);
        }
    }

    /// A repeating primitive, with an aligned `_key` array when any entry
    /// has element parts.
    pub fn prim_list<T: PrimitiveValue>(&mut self, key: &str, v: &[Primitive<T>]) {
        if v.is_empty() {
            return;
        }
        if v.iter().any(|p| p.value.is_some()) {
            let vals = v
                .iter()
                .map(|p| p.value.as_ref().map_or(Json::Null, PrimitiveValue::to_json))
                .collect();
            self.raw(key, Json::Array(vals));
        }
        if v.iter().any(|p| p.id.is_some() || !p.extension.is_empty()) {
            let parts = v
                .iter()
                .map(|p| parts_json(p).unwrap_or(Json::Null))
                .collect();
            self.raw(&format!("_{key}"), Json::Array(parts));
        }
    }

    /// A choice element, if set.
    pub fn choice<C: Choice>(&mut self, base: &str, v: &Option<C>) {
        if let Some(c) = v {
            c.encode_into(base, self);
        }
    }

    /// The finished object.
    pub fn finish(self) -> Json {
        Json::Object(self.members)
    }
}

fn parts_json<T>(p: &Primitive<T>) -> Option<Json> {
    if p.id.is_none() && p.extension.is_empty() {
        return None;
    }
    let mut w = ObjWriter::new();
    w.string("id", &p.id);
    w.list("extension", &p.extension);
    Some(w.finish())
}

/// Take and check a resource's `resourceType`.
pub fn resource_type(o: &mut Obj<'_>, d: &mut Decoder, expected: &str) {
    match o.take("resourceType") {
        Some(Json::String(s)) if s == expected => {}
        Some(other) => d.at("resourceType", |d| {
            d.error(format!(
                "expected \"{expected}\", found {}",
                other.to_compact()
            ))
        }),
        None => d.at("resourceType", |d| d.error("required element missing")),
    }
}

impl<T: FhirJson> FhirJson for Box<T> {
    const TYPE_NAME: &'static str = T::TYPE_NAME;

    fn decode(v: &Json, d: &mut Decoder) -> Option<Self> {
        T::decode(v, d).map(Box::new)
    }

    fn encode(&self) -> Json {
        (**self).encode()
    }
}

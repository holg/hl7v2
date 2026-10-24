//! Generates `crates/fhirkit/src/r4/` from the FHIR R4 core package.
//!
//! ```text
//! cargo run -p fhirkit-gen -- <package-dir> <output-dir>
//! cargo run -p fhirkit-gen -- tmp/fhir/core/package crates/fhirkit/src/r4
//! ```
//!
//! `<package-dir>` is the unpacked `package/` folder of `hl7.fhir.r4.core`
//! 4.0.1 (from <https://packages.fhir.org/hl7.fhir.r4.core/4.0.1>). The
//! output is committed, so users of fhirkit never need the package.
//!
//! Every struct is flattened (a resource carries `id`, `meta`, `text` ...
//! itself, no inheritance), every singular element is an `Option`, every
//! repeating one a `Vec`, `[x]` elements become enums, and fields on a
//! by-value type cycle (`Identifier.assigner` / `Reference.identifier`) are
//! boxed.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, String>;

fn main() {
    if let Err(e) = run() {
        eprintln!("fhirkit-gen: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [package, out] = args.as_slice() else {
        return Err("usage: fhirkit-gen <package-dir> <output-dir>".into());
    };
    let model = Model::load(Path::new(package))?;
    model.write(Path::new(out))?;
    eprintln!(
        "fhirkit-gen: {} datatypes, {} resources, {} structs, {} choice enums",
        model.datatypes.len(),
        model.resources.len(),
        model.structs.len(),
        model.choices.len()
    );
    Ok(())
}

// --- model -----------------------------------------------------------------

#[derive(Debug, Clone)]
enum FieldType {
    /// `System.String`: element and resource ids, extension urls.
    PlainString,
    /// A primitive: Rust value type and fhirkit `Kind` variant.
    Prim {
        rust: &'static str,
        kind: &'static str,
    },
    /// A struct (datatype or backbone) or the `Resource` enum.
    Complex(String),
    /// A choice enum.
    Choice(String),
}

#[derive(Debug, Clone)]
struct Field {
    json: String,
    rust: String,
    min: u32,
    many: bool,
    ty: FieldType,
    boxed: bool,
    doc: String,
}

#[derive(Debug, Clone)]
struct Struct {
    name: String,
    /// FHIR path (`Observation.referenceRange`).
    path: String,
    doc: String,
    fields: Vec<Field>,
    /// The resource type, for resource structs.
    resource: Option<String>,
    feature: Option<String>,
}

#[derive(Debug, Clone)]
struct Variant {
    code: String,
    name: String,
    ty: FieldType,
    boxed: bool,
}

#[derive(Debug, Clone)]
struct ChoiceEnum {
    name: String,
    path: String,
    variants: Vec<Variant>,
    feature: Option<String>,
}

struct Model {
    datatypes: Vec<String>,
    /// (type, feature, doc)
    resources: Vec<(String, String, String)>,
    structs: Vec<Struct>,
    choices: Vec<ChoiceEnum>,
}

fn prim(code: &str) -> Option<FieldType> {
    let (rust, kind) = match code {
        "boolean" => ("bool", "Boolean"),
        "integer" => ("i32", "Integer"),
        "positiveInt" => ("i32", "PositiveInt"),
        "unsignedInt" => ("i32", "UnsignedInt"),
        "decimal" => ("Decimal", "Decimal"),
        "string" => ("String", "String"),
        "code" => ("String", "Code"),
        "id" => ("String", "Id"),
        "markdown" => ("String", "Markdown"),
        "uri" => ("String", "Uri"),
        "url" => ("String", "Url"),
        "canonical" => ("String", "Canonical"),
        "oid" => ("String", "Oid"),
        "uuid" => ("String", "Uuid"),
        "base64Binary" => ("String", "Base64Binary"),
        "date" => ("String", "Date"),
        "dateTime" => ("String", "DateTime"),
        "instant" => ("String", "Instant"),
        "time" => ("String", "Time"),
        "xhtml" => ("String", "Xhtml"),
        _ => return None,
    };
    Some(FieldType::Prim { rust, kind })
}

fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// `Observation.referenceRange` -> `ObservationReferenceRange`.
fn camel_path(path: &str) -> String {
    path.split('.').map(upper_first).collect()
}

const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in",
    "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// `modifierExtension` -> `modifier_extension`, keywords get a `_` suffix.
fn snake(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1);
            let boundary = prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                || (prev.is_some_and(char::is_uppercase) && next.is_some_and(|n| n.is_lowercase()));
            if boundary {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    if KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

fn doc_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn category(sd: &Value) -> Option<String> {
    sd.get("extension")?
        .as_array()?
        .iter()
        .find(|e| str_of(e, "url").is_some_and(|u| u.ends_with("structuredefinition-category")))
        .and_then(|e| str_of(e, "valueString"))
        .and_then(|c| c.split('.').next())
        .map(str::to_lowercase)
}

impl Model {
    fn load(dir: &Path) -> Result<Model> {
        let mut sds: BTreeMap<String, Value> = BTreeMap::new();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("StructureDefinition-") || !name.ends_with(".json") {
                continue;
            }
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let sd: Value =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            let kind = str_of(&sd, "kind").unwrap_or("");
            let derivation = str_of(&sd, "derivation").unwrap_or("");
            let abstract_ = sd.get("abstract").and_then(Value::as_bool).unwrap_or(false);
            if derivation != "specialization" || abstract_ {
                continue;
            }
            if kind == "complex-type" || kind == "resource" {
                let ty = str_of(&sd, "type").ok_or("StructureDefinition without type")?;
                sds.insert(ty.to_string(), sd);
            }
        }

        let mut model = Model {
            datatypes: Vec::new(),
            resources: Vec::new(),
            structs: Vec::new(),
            choices: Vec::new(),
        };
        for (ty, sd) in &sds {
            let is_resource = str_of(sd, "kind") == Some("resource");
            let feature = if is_resource {
                Some(category(sd).ok_or_else(|| format!("{ty}: no category"))?)
            } else {
                None
            };
            let doc = doc_line(str_of(sd, "description").unwrap_or(""));
            if is_resource {
                model.resources.push((
                    ty.clone(),
                    feature.clone().unwrap_or_default(),
                    doc.clone(),
                ));
            } else {
                model.datatypes.push(ty.clone());
            }
            model.add_type(ty, sd, is_resource, feature, doc)?;
        }
        model.box_cycles();
        let mut names = BTreeSet::new();
        for n in model
            .structs
            .iter()
            .map(|s| &s.name)
            .chain(model.choices.iter().map(|c| &c.name))
        {
            if !names.insert(n.clone()) {
                return Err(format!("generated name {n} is used twice"));
            }
        }
        Ok(model)
    }

    fn add_type(
        &mut self,
        ty: &str,
        sd: &Value,
        is_resource: bool,
        feature: Option<String>,
        doc: String,
    ) -> Result<()> {
        let elements: Vec<&Value> = sd
            .pointer("/snapshot/element")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{ty}: no snapshot"))?
            .iter()
            .filter(|e| e.get("sliceName").is_none())
            .collect();
        let paths: BTreeSet<&str> = elements.iter().filter_map(|e| str_of(e, "path")).collect();
        let has_children = |p: &str| {
            let prefix = format!("{p}.");
            paths.iter().any(|q| q.starts_with(&prefix))
        };

        // Group elements under their parent path, in snapshot order.
        let mut owners: Vec<(String, String)> = vec![(ty.to_string(), doc)];
        let mut children: HashMap<String, Vec<&Value>> = HashMap::new();
        for e in &elements[1..] {
            let path = str_of(e, "path").unwrap_or("");
            let (parent, _) = path
                .rsplit_once('.')
                .ok_or_else(|| format!("{ty}: bad path {path}"))?;
            children.entry(parent.to_string()).or_default().push(e);
            if has_children(path) && e.get("contentReference").is_none() {
                owners.push((path.to_string(), doc_line(str_of(e, "short").unwrap_or(""))));
            }
        }

        for (owner, owner_doc) in owners {
            let name = if owner == ty {
                ty.to_string()
            } else {
                camel_path(&owner)
            };
            let mut fields = Vec::new();
            for e in children.get(&owner).map(Vec::as_slice).unwrap_or(&[]) {
                let path = str_of(e, "path").unwrap_or("");
                let raw_name = path.rsplit('.').next().unwrap_or("");
                let min = e.get("min").and_then(Value::as_u64).unwrap_or(0) as u32;
                let max = str_of(e, "max").unwrap_or("1");
                let many = max != "0" && max != "1";
                let short = doc_line(str_of(e, "short").unwrap_or(""));
                let codes: Vec<&str> = e
                    .get("type")
                    .and_then(Value::as_array)
                    .map(|ts| ts.iter().filter_map(|t| str_of(t, "code")).collect())
                    .unwrap_or_default();

                let (json, ty_) = if let Some(base) = raw_name.strip_suffix("[x]") {
                    let enum_name = camel_path(&format!("{owner}.{base}"));
                    let variants = codes
                        .iter()
                        .map(|c| {
                            Ok(Variant {
                                code: c.to_string(),
                                name: upper_first(c),
                                ty: self.type_of(c, path, &name)?,
                                boxed: false,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    self.choices.push(ChoiceEnum {
                        name: enum_name.clone(),
                        path: format!("{owner}.{base}[x]"),
                        variants,
                        feature: feature.clone(),
                    });
                    (base.to_string(), FieldType::Choice(enum_name))
                } else if let Some(r) = str_of(e, "contentReference") {
                    (
                        raw_name.to_string(),
                        FieldType::Complex(camel_path(r.trim_start_matches('#'))),
                    )
                } else if has_children(path) {
                    (raw_name.to_string(), FieldType::Complex(camel_path(path)))
                } else {
                    let [code] = codes.as_slice() else {
                        return Err(format!("{path}: {} types without [x]", codes.len()));
                    };
                    (raw_name.to_string(), self.type_of(code, path, &name)?)
                };
                fields.push(Field {
                    rust: snake(&json),
                    json,
                    min,
                    many,
                    ty: ty_,
                    boxed: false,
                    doc: short,
                });
            }
            self.structs.push(Struct {
                name,
                path: owner.clone(),
                doc: owner_doc,
                fields,
                resource: (is_resource && owner == ty).then(|| ty.to_string()),
                feature: feature.clone(),
            });
        }
        Ok(())
    }

    fn type_of(&self, code: &str, path: &str, _owner: &str) -> Result<FieldType> {
        if code.starts_with("http://hl7.org/fhirpath/System.") {
            return Ok(FieldType::PlainString);
        }
        if let Some(p) = prim(code) {
            return Ok(p);
        }
        if code.chars().next().is_some_and(char::is_uppercase) {
            return Ok(FieldType::Complex(code.to_string()));
        }
        Err(format!("{path}: unknown type code {code}"))
    }

    /// Box every by-value edge that lies on a cycle.
    fn box_cycles(&mut self) {
        let mut edges: HashMap<String, Vec<String>> = HashMap::new();
        for s in &self.structs {
            for f in s.fields.iter().filter(|f| !f.many) {
                if let FieldType::Complex(t) | FieldType::Choice(t) = &f.ty {
                    if t != "Resource" {
                        edges.entry(s.name.clone()).or_default().push(t.clone());
                    }
                }
            }
        }
        for c in &self.choices {
            for v in &c.variants {
                if let FieldType::Complex(t) = &v.ty {
                    edges.entry(c.name.clone()).or_default().push(t.clone());
                }
            }
        }
        let reaches = |from: &str, to: &str| -> bool {
            let mut stack = vec![from.to_string()];
            let mut seen = BTreeSet::new();
            while let Some(n) = stack.pop() {
                if n == to {
                    return true;
                }
                if seen.insert(n.clone()) {
                    stack.extend(edges.get(&n).cloned().unwrap_or_default());
                }
            }
            false
        };
        let mut boxes = Vec::new();
        for s in &self.structs {
            for (i, f) in s.fields.iter().enumerate().filter(|(_, f)| !f.many) {
                if let FieldType::Complex(t) | FieldType::Choice(t) = &f.ty {
                    if t != "Resource" && reaches(t, &s.name) {
                        boxes.push((s.name.clone(), i));
                    }
                }
            }
        }
        for (name, i) in boxes {
            if let Some(s) = self.structs.iter_mut().find(|s| s.name == name) {
                s.fields[i].boxed = true;
            }
        }
        let mut vboxes = Vec::new();
        for c in &self.choices {
            for (i, v) in c.variants.iter().enumerate() {
                if let FieldType::Complex(t) = &v.ty {
                    if reaches(t, &c.name) {
                        vboxes.push((c.name.clone(), i));
                    }
                }
            }
        }
        for (name, i) in vboxes {
            if let Some(c) = self.choices.iter_mut().find(|c| c.name == name) {
                c.variants[i].boxed = true;
            }
        }
    }
}

// --- output ----------------------------------------------------------------

const HEADER: &str = "// @generated by fhirkit-gen from hl7.fhir.r4.core 4.0.1. Do not edit.\n";

fn cfg(feature: &Option<String>) -> String {
    feature
        .as_ref()
        .map(|f| format!("#[cfg(feature = \"{f}\")]\n"))
        .unwrap_or_default()
}

fn rust_type(f: &Field) -> String {
    let inner = match &f.ty {
        FieldType::PlainString => "String".to_string(),
        FieldType::Prim { rust, .. } => format!("Primitive<{rust}>"),
        FieldType::Complex(t) | FieldType::Choice(t) => t.clone(),
    };
    let inner = if f.boxed {
        format!("Box<{inner}>")
    } else {
        inner
    };
    if f.many {
        format!("Vec<{inner}>")
    } else {
        format!("Option<{inner}>")
    }
}

fn decode_expr(f: &Field) -> String {
    let key = &f.json;
    match (&f.ty, f.many) {
        (FieldType::PlainString, false) => format!("o.string(d, \"{key}\")"),
        (FieldType::PlainString, true) => format!("o.prim_list::<String>(d, \"{key}\", Kind::String).into_iter().filter_map(|p| p.value).collect()"),
        (FieldType::Prim { kind, .. }, false) => format!("o.prim(d, \"{key}\", Kind::{kind})"),
        (FieldType::Prim { kind, .. }, true) => format!("o.prim_list(d, \"{key}\", Kind::{kind})"),
        (FieldType::Complex(_), false) => format!("o.one(d, \"{key}\")"),
        (FieldType::Complex(_), true) => format!("o.list(d, \"{key}\")"),
        (FieldType::Choice(_), _) => format!("o.choice(d, \"{key}\")"),
    }
}

fn encode_stmt(f: &Field) -> String {
    let (key, r) = (&f.json, &f.rust);
    match (&f.ty, f.many) {
        (FieldType::PlainString, false) => format!("w.string(\"{key}\", &self.{r});"),
        (FieldType::PlainString, true) => format!(
            "if !self.{r}.is_empty() {{ w.raw(\"{key}\", Json::Array(self.{r}.iter().map(|s| Json::String(s.clone())).collect())); }}"
        ),
        (FieldType::Prim { .. }, false) => format!("w.prim(\"{key}\", &self.{r});"),
        (FieldType::Prim { .. }, true) => format!("w.prim_list(\"{key}\", &self.{r});"),
        (FieldType::Complex(_), false) => format!("w.one(\"{key}\", &self.{r});"),
        (FieldType::Complex(_), true) => format!("w.list(\"{key}\", &self.{r});"),
        (FieldType::Choice(_), _) => format!("w.choice(\"{key}\", &self.{r});"),
    }
}

fn write_struct(out: &mut String, s: &Struct) {
    let c = cfg(&s.feature);
    let _ = writeln!(
        out,
        "/// {}",
        if s.doc.is_empty() { &s.path } else { &s.doc }
    );
    let _ = writeln!(out, "///\n/// FHIR path: `{}`.", s.path);
    let _ = write!(
        out,
        "{c}#[derive(Debug, Clone, PartialEq, Default)]\npub struct {} {{\n",
        s.name
    );
    for f in &s.fields {
        let card = format!("{}..{}", f.min, if f.many { "*" } else { "1" });
        let name = if matches!(f.ty, FieldType::Choice(_)) {
            format!("{}[x]", f.json)
        } else {
            f.json.clone()
        };
        let _ = writeln!(out, "    /// `{name}` ({card}): {}", f.doc);
        let _ = writeln!(out, "    pub {}: {},", f.rust, rust_type(f));
    }
    out.push_str("}\n\n");

    let type_name = s.resource.clone().unwrap_or_else(|| s.name.clone());
    let _ = write!(
        out,
        "{c}impl FhirJson for {} {{\n    const TYPE_NAME: &'static str = \"{type_name}\";\n\n    fn decode(v: &Json, d: &mut Decoder) -> Option<Self> {{\n        let mut o = Obj::new(v, d)?;\n",
        s.name
    );
    if let Some(rt) = &s.resource {
        let _ = writeln!(out, "        resource_type(&mut o, d, \"{rt}\");");
    }
    let _ = writeln!(out, "        let r = {} {{", s.name);
    for f in &s.fields {
        let _ = writeln!(out, "            {}: {},", f.rust, decode_expr(f));
    }
    out.push_str("        };\n");
    for f in s.fields.iter().filter(|f| f.min > 0) {
        if f.many {
            let _ = writeln!(
                out,
                "        required_list(d, \"{}\", &r.{}, {});",
                f.json, f.rust, f.min
            );
        } else {
            let key = if matches!(f.ty, FieldType::Choice(_)) {
                format!("{}[x]", f.json)
            } else {
                f.json.clone()
            };
            let _ = writeln!(out, "        required(d, \"{key}\", &r.{});", f.rust);
        }
    }
    out.push_str("        o.finish(d);\n        Some(r)\n    }\n\n    fn encode(&self) -> Json {\n        let mut w = ObjWriter::new();\n");
    if let Some(rt) = &s.resource {
        let _ = writeln!(
            out,
            "        w.raw(\"resourceType\", Json::String(\"{rt}\".into()));"
        );
    }
    for f in &s.fields {
        let _ = writeln!(out, "        {}", encode_stmt(f));
    }
    out.push_str("        w.finish()\n    }\n}\n\n");
}

fn write_choice(out: &mut String, c: &ChoiceEnum) {
    let cf = cfg(&c.feature);
    let _ = writeln!(out, "/// The types `{}` allows.", c.path);
    let _ = write!(
        out,
        "{cf}#[derive(Debug, Clone, PartialEq)]\npub enum {} {{\n",
        c.name
    );
    for v in &c.variants {
        let inner = match &v.ty {
            FieldType::Prim { rust, .. } => format!("Primitive<{rust}>"),
            FieldType::Complex(t) => t.clone(),
            FieldType::PlainString => "String".into(),
            FieldType::Choice(t) => t.clone(),
        };
        let inner = if v.boxed {
            format!("Box<{inner}>")
        } else {
            inner
        };
        let _ = writeln!(out, "    /// `{}`\n    {}({inner}),", v.code, v.name);
    }
    out.push_str("}\n\n");
    let codes: Vec<String> = c
        .variants
        .iter()
        .map(|v| format!("\"{}\"", v.code))
        .collect();
    let _ = write!(
        out,
        "{cf}impl Choice for {} {{\n    const TYPES: &'static [&'static str] = &[{}];\n\n    fn decode_as(code: &str, key: &str, o: &mut Obj<'_>, d: &mut Decoder) -> Option<Self> {{\n        match code {{\n",
        c.name,
        codes.join(", ")
    );
    for v in &c.variants {
        let expr = match &v.ty {
            FieldType::Prim { kind, .. } => {
                format!("o.prim(d, key, Kind::{kind}).map(Self::{})", v.name)
            }
            _ => format!("o.one(d, key).map(Self::{})", v.name),
        };
        let _ = writeln!(out, "            \"{}\" => {expr},", v.code);
    }
    out.push_str("            _ => None,\n        }\n    }\n\n    fn encode_into(&self, base: &str, w: &mut ObjWriter) {\n        match self {\n");
    for v in &c.variants {
        let call = match &v.ty {
            FieldType::Prim { .. } => "prim_value",
            _ => "value",
        };
        let _ = writeln!(
            out,
            "            Self::{}(v) => w.{call}(&choice_key(base, \"{}\"), v),",
            v.name, v.code
        );
    }
    out.push_str("        }\n    }\n}\n\n");
}

const PRELUDE: &str = "#![allow(unused_imports, clippy::large_enum_variant, clippy::too_many_lines, clippy::doc_markdown)]\n\nuse crate::decode::{choice_key, required, required_list, resource_type, Choice, Decoder, FhirJson, Obj, ObjWriter};\nuse crate::json::Json;\nuse crate::primitive::{Decimal, Kind, Primitive};\nuse crate::r4::*;\n\n";

impl Model {
    fn write(&self, out_dir: &Path) -> Result<()> {
        let res_dir = out_dir.join("resources");
        std::fs::create_dir_all(&res_dir).map_err(|e| format!("{}: {e}", res_dir.display()))?;
        let write = |p: PathBuf, s: String| {
            std::fs::write(&p, s).map_err(|e| format!("{}: {e}", p.display()))
        };

        // Datatypes: every struct and choice without a feature.
        let mut dt = format!("{HEADER}//! FHIR R4 datatypes.\n\n{PRELUDE}");
        for s in self.structs.iter().filter(|s| s.feature.is_none()) {
            write_struct(&mut dt, s);
        }
        for c in self.choices.iter().filter(|c| c.feature.is_none()) {
            write_choice(&mut dt, c);
        }
        write(out_dir.join("datatypes.rs"), dt)?;

        // One file per resource, with its backbones and choices.
        let mut res_mod =
            format!("{HEADER}//! FHIR R4 resources, one module each, grouped by feature.\n\n");
        for (ty, feature, _) in &self.resources {
            let module = snake(ty);
            let mut src = format!("{HEADER}//! The FHIR R4 `{ty}` resource.\n\n{PRELUDE}");
            let prefix = format!("{ty}.");
            for s in self
                .structs
                .iter()
                .filter(|s| s.path == *ty || s.path.starts_with(&prefix))
            {
                write_struct(&mut src, s);
            }
            for c in self.choices.iter().filter(|c| c.path.starts_with(&prefix)) {
                write_choice(&mut src, c);
            }
            write(res_dir.join(format!("{module}.rs")), src)?;
            let _ = writeln!(res_mod, "#[cfg(feature = \"{feature}\")]\nmod {module};\n#[cfg(feature = \"{feature}\")]\npub use {module}::*;");
        }
        write(res_dir.join("mod.rs"), res_mod)?;

        // The Resource enum.
        let mut r = format!("{HEADER}//! Any FHIR R4 resource, by `resourceType`.\n\n#![allow(clippy::large_enum_variant)]\n\nuse crate::decode::{{Decoder, FhirJson}};\nuse crate::json::Json;\nuse crate::r4::*;\n\n");
        r.push_str("/// Any resource, as `contained`, `Bundle.entry.resource` or a document root.\n#[derive(Debug, Clone, PartialEq)]\n#[non_exhaustive]\npub enum Resource {\n");
        for (ty, feature, doc) in &self.resources {
            let first = doc.split(". ").next().unwrap_or("");
            let _ = writeln!(
                r,
                "    /// {first}\n    #[cfg(feature = \"{feature}\")]\n    {ty}(Box<{ty}>),"
            );
        }
        r.push_str("}\n\nimpl Resource {\n    /// The `resourceType`.\n    pub fn resource_type(&self) -> &'static str {\n        match self {\n");
        for (ty, feature, _) in &self.resources {
            let _ = writeln!(r, "            #[cfg(feature = \"{feature}\")]\n            Resource::{ty}(_) => \"{ty}\",");
        }
        r.push_str("        }\n    }\n\n    /// The resource's logical id.\n    pub fn id(&self) -> Option<&str> {\n        match self {\n");
        for (ty, feature, _) in &self.resources {
            let _ = writeln!(r, "            #[cfg(feature = \"{feature}\")]\n            Resource::{ty}(x) => x.id.as_deref(),");
        }
        r.push_str("        }\n    }\n}\n\nimpl FhirJson for Resource {\n    const TYPE_NAME: &'static str = \"Resource\";\n\n    fn decode(v: &Json, d: &mut Decoder) -> Option<Self> {\n        let Some(rt) = v.get(\"resourceType\").and_then(Json::as_str) else {\n            d.error(\"a resource needs a resourceType\");\n            return None;\n        };\n        match rt {\n");
        for (ty, feature, _) in &self.resources {
            let _ = writeln!(r, "            #[cfg(feature = \"{feature}\")]\n            \"{ty}\" => {ty}::decode(v, d).map(|x| Resource::{ty}(Box::new(x))),");
        }
        r.push_str("            other => {\n                d.error(format!(\"{other} is not a FHIR R4 resource type this build knows\"));\n                None\n            }\n        }\n    }\n\n    fn encode(&self) -> Json {\n        match self {\n");
        for (ty, feature, _) in &self.resources {
            let _ = writeln!(r, "            #[cfg(feature = \"{feature}\")]\n            Resource::{ty}(x) => x.encode(),");
        }
        r.push_str("        }\n    }\n}\n\n/// Every resource type in FHIR R4, whether this build includes it or not.\npub const RESOURCE_TYPES: &[&str] = &[\n");
        for (ty, _, _) in &self.resources {
            let _ = writeln!(r, "    \"{ty}\",");
        }
        r.push_str("];\n");
        write(out_dir.join("resource.rs"), r)?;

        let m = format!(
            "{HEADER}//! FHIR R4 (4.0.1): {} resources and {} datatypes, generated from the\n//! official specification package.\n\nmod datatypes;\nmod resource;\nmod resources;\n\npub use datatypes::*;\npub use resource::*;\npub use resources::*;\n",
            self.resources.len(),
            self.datatypes.len()
        );
        write(out_dir.join("mod.rs"), m)?;
        Ok(())
    }
}

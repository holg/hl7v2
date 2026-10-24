//! PyO3 bindings for fhirkit. Thin by design: no logic here, only
//! conversion between Python values and fhirkit's JSON and resources.
//!
//! Decimals cross the boundary as `decimal.Decimal`, never as `float`, so
//! `1.50` stays `1.50` both ways.

use fhirkit::r4::Resource;
use fhirkit::{FhirJson, Issue, Json, Severity};
use pyo3::create_exception;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

create_exception!(fhirkit, ParseError, PyValueError, "The input is not JSON.");
create_exception!(
    fhirkit,
    ValidationError,
    PyValueError,
    "The input is not valid FHIR R4; `.issues` lists every problem."
);

/// One problem found while reading, with the element's path.
#[pyclass(name = "Issue", frozen, get_all, skip_from_py_object)]
#[derive(Clone)]
pub struct PyIssue {
    /// "error" or "warning".
    pub severity: String,
    /// Where, e.g. "Patient.name[0].given[1]".
    pub path: String,
    /// What is wrong.
    pub message: String,
}

impl From<&Issue> for PyIssue {
    fn from(i: &Issue) -> Self {
        PyIssue {
            severity: match i.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            }
            .to_string(),
            path: i.path.clone(),
            message: i.message.clone(),
        }
    }
}

#[pymethods]
impl PyIssue {
    fn __str__(&self) -> String {
        format!("{} at {}: {}", self.severity, self.path, self.message)
    }

    fn __repr__(&self) -> String {
        format!(
            "<fhirkit.Issue {} {}: {}>",
            self.severity, self.path, self.message
        )
    }
}

/// A FHIR R4 resource, read and checked.
#[pyclass(name = "Resource", frozen)]
pub struct PyResource {
    inner: Resource,
    issues: Vec<Issue>,
}

impl PyResource {
    /// The wrapped resource, for other extension modules built on fhirkit.
    pub fn inner(&self) -> &Resource {
        &self.inner
    }
}

#[pymethods]
impl PyResource {
    /// The resourceType, e.g. "Patient".
    #[getter]
    fn resource_type(&self) -> &'static str {
        self.inner.resource_type()
    }

    /// The logical id, if any.
    #[getter]
    fn id(&self) -> Option<String> {
        self.inner.id().map(str::to_string)
    }

    /// Problems found while reading (empty for valid input).
    #[getter]
    fn issues(&self) -> Vec<PyIssue> {
        self.issues.iter().map(PyIssue::from).collect()
    }

    /// FHIR JSON text, in definition order.
    #[pyo3(signature = (pretty = false))]
    fn to_json(&self, pretty: bool) -> String {
        let j = self.inner.encode();
        if pretty {
            j.to_pretty()
        } else {
            j.to_compact()
        }
    }

    /// The resource as plain Python values; decimals as decimal.Decimal.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        json_to_py(py, &self.inner.encode())
    }

    fn __repr__(&self) -> String {
        let issues = match self.issues.len() {
            0 => String::new(),
            n => format!(" ({n} issue{})", if n == 1 { "" } else { "s" }),
        };
        match self.inner.id() {
            Some(id) => format!(
                "<fhirkit.Resource {}/{id}{issues}>",
                self.inner.resource_type()
            ),
            None => format!("<fhirkit.Resource {}{issues}>", self.inner.resource_type()),
        }
    }
}

fn json_to_py<'py>(py: Python<'py>, v: &Json) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        Json::Null => py.None().into_bound(py),
        Json::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
        Json::Number(n) if !n.contains(['.', 'e', 'E']) => {
            py.get_type::<PyInt>().call1((n.as_str(),))?
        }
        Json::Number(n) => py
            .import("decimal")?
            .getattr("Decimal")?
            .call1((n.as_str(),))?,
        Json::String(s) => PyString::new(py, s).into_any(),
        Json::Array(a) => {
            let items = a
                .iter()
                .map(|x| json_to_py(py, x))
                .collect::<PyResult<Vec<_>>>()?;
            PyList::new(py, items)?.into_any()
        }
        Json::Object(m) => {
            let d = PyDict::new(py);
            for (k, x) in m {
                d.set_item(k, json_to_py(py, x)?)?;
            }
            d.into_any()
        }
    })
}

fn py_to_json(v: &Bound<'_, PyAny>, depth: usize) -> PyResult<Json> {
    if depth > fhirkit::json::MAX_DEPTH {
        return Err(PyValueError::new_err("nested too deeply"));
    }
    if v.is_none() {
        return Ok(Json::Null);
    }
    // bool before int: in Python, True is an int.
    if let Ok(b) = v.cast::<PyBool>() {
        return Ok(Json::Bool(b.is_true()));
    }
    if v.cast::<PyInt>().is_ok() {
        return Ok(Json::Number(v.str()?.to_string()));
    }
    if v.cast::<PyFloat>().is_ok() {
        let f: f64 = v.extract()?;
        if !f.is_finite() {
            return Err(PyValueError::new_err(
                "NaN and infinity are not JSON numbers",
            ));
        }
        return Ok(Json::Number(v.repr()?.to_string()));
    }
    if let Ok(s) = v.cast::<PyString>() {
        return Ok(Json::String(s.to_string()));
    }
    let decimal = v.py().import("decimal")?.getattr("Decimal")?;
    if v.is_instance(&decimal)? {
        if !v.call_method0("is_finite")?.is_truthy()? {
            return Err(PyValueError::new_err(
                "NaN and infinity are not JSON numbers",
            ));
        }
        // str(Decimal) keeps the exponent form Python chose; FHIR accepts it.
        return Ok(Json::Number(v.str()?.to_string()));
    }
    if let Ok(d) = v.cast::<PyDict>() {
        let mut members = Vec::with_capacity(d.len());
        for (k, x) in d.iter() {
            let key = k
                .cast::<PyString>()
                .map_err(|_| PyTypeError::new_err("object keys must be str"))?
                .to_string();
            members.push((key, py_to_json(&x, depth + 1)?));
        }
        return Ok(Json::Object(members));
    }
    if v.cast::<PyList>().is_ok() || v.cast::<PyTuple>().is_ok() {
        let items = v
            .try_iter()?
            .map(|x| py_to_json(&x?, depth + 1))
            .collect::<PyResult<Vec<_>>>()?;
        return Ok(Json::Array(items));
    }
    Err(PyTypeError::new_err(format!(
        "{} is not a JSON value",
        v.get_type().name()?
    )))
}

/// str, bytes or dict to parsed JSON.
fn to_json_value(data: &Bound<'_, PyAny>) -> PyResult<Json> {
    let text = if let Ok(b) = data.cast::<PyBytes>() {
        String::from_utf8(b.as_bytes().to_vec())
            .map_err(|e| ParseError::new_err(format!("not UTF-8: {e}")))?
    } else if let Ok(s) = data.cast::<PyString>() {
        s.to_string()
    } else {
        return py_to_json(data, 0);
    };
    data.py()
        .detach(move || Json::parse(&text))
        .map_err(|e| ParseError::new_err(e.to_string()))
}

fn issues_list(py: Python<'_>, issues: &[Issue]) -> PyResult<Py<PyList>> {
    let items = issues
        .iter()
        .map(|i| Py::new(py, PyIssue::from(i)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, items)?.unbind())
}

fn validation_error(py: Python<'_>, what: &str, issues: &[Issue]) -> PyErr {
    let mut msg = format!("{what}: {} problem(s)", issues.len());
    for i in issues.iter().take(10) {
        msg.push_str(&format!("\n  {i}"));
    }
    if issues.len() > 10 {
        msg.push_str(&format!("\n  ... and {} more", issues.len() - 10));
    }
    let err = ValidationError::new_err(msg);
    if let Ok(list) = issues_list(py, issues) {
        let _ = err.value(py).setattr("issues", list);
    }
    err
}

/// Read a FHIR R4 resource from JSON text (str or bytes) or a dict.
///
/// Problems are collected in `Resource.issues`. With `strict=True`, any
/// error-level problem raises ValidationError instead. Input that is not
/// JSON raises ParseError; JSON that is not a resource at all raises
/// ValidationError.
#[pyfunction]
#[pyo3(signature = (data, strict = false))]
fn parse(py: Python<'_>, data: Bound<'_, PyAny>, strict: bool) -> PyResult<PyResource> {
    let json = to_json_value(&data)?;
    let (resource, issues) = py.detach(move || fhirkit::decode_json::<Resource>(&json));
    let errors: Vec<Issue> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .cloned()
        .collect();
    match resource {
        None => Err(validation_error(py, "not a FHIR R4 resource", &issues)),
        Some(_) if strict && !errors.is_empty() => {
            Err(validation_error(py, "invalid FHIR R4", &errors))
        }
        Some(inner) => Ok(PyResource { inner, issues }),
    }
}

/// Every problem in a FHIR R4 resource, as a list of Issue (empty when
/// valid). Input that is not JSON raises ParseError.
#[pyfunction]
fn validate(py: Python<'_>, data: Bound<'_, PyAny>) -> PyResult<Vec<PyIssue>> {
    let json = to_json_value(&data)?;
    let (_, issues) = py.detach(move || fhirkit::decode_json::<Resource>(&json));
    Ok(issues.iter().map(PyIssue::from).collect())
}

/// Read many resources at once on Rust threads, with the GIL released.
///
/// `items` is a list of str, bytes or dict. Returns a list of Resource in
/// the same order. `threads=0` uses one thread per core. An item that is
/// not JSON raises ParseError, one that is not a resource (or, with
/// `strict=True`, has any error) raises ValidationError; both name the
/// item's index.
#[pyfunction]
#[pyo3(signature = (items, threads = 0, strict = false))]
fn parse_many(
    py: Python<'_>,
    items: Vec<Bound<'_, PyAny>>,
    threads: usize,
    strict: bool,
) -> PyResult<Vec<PyResource>> {
    // Text for every item while holding the GIL; dicts are written out.
    let texts = items
        .iter()
        .enumerate()
        .map(|(i, x)| {
            if let Ok(b) = x.cast::<PyBytes>() {
                String::from_utf8(b.as_bytes().to_vec())
                    .map_err(|e| ParseError::new_err(format!("item {i}: not UTF-8: {e}")))
            } else if let Ok(s) = x.cast::<PyString>() {
                Ok(s.to_string())
            } else {
                py_to_json(x, 0).map(|j| j.to_compact())
            }
        })
        .collect::<PyResult<Vec<String>>>()?;
    let read = py.detach(move || {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        fhirkit::from_json_many::<Resource>(&refs, threads)
    });
    read.into_iter()
        .enumerate()
        .map(|(i, r)| {
            let (resource, issues) =
                r.map_err(|e| ParseError::new_err(format!("item {i}: {e}")))?;
            let errors: Vec<Issue> = issues
                .iter()
                .filter(|x| x.severity == Severity::Error)
                .cloned()
                .collect();
            match resource {
                None => Err(validation_error(
                    py,
                    &format!("item {i}: not a FHIR R4 resource"),
                    &issues,
                )),
                Some(_) if strict && !errors.is_empty() => Err(validation_error(
                    py,
                    &format!("item {i}: invalid FHIR R4"),
                    &errors,
                )),
                Some(inner) => Ok(PyResource { inner, issues }),
            }
        })
        .collect()
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyResource>()?;
    m.add_class::<PyIssue>()?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(parse_many, m)?)?;
    m.add_function(wrap_pyfunction!(validate, m)?)?;
    m.add("ParseError", m.py().get_type::<ParseError>())?;
    m.add("ValidationError", m.py().get_type::<ValidationError>())?;
    m.add("RESOURCE_TYPES", fhirkit::r4::RESOURCE_TYPES.to_vec())?;
    m.add("FHIR_VERSION", "4.0.1")?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_and_ints_round_trip_through_python() {
        Python::initialize();
        Python::attach(|py| {
            let text = r#"{"a": 1.50, "b": 12345678901234567890, "c": [true, null, "x"]}"#;
            let j = Json::parse(text).unwrap();
            let obj = json_to_py(py, &j).unwrap();
            let back = py_to_json(&obj, 0).unwrap();
            assert_eq!(
                back.to_compact(),
                r#"{"a":1.50,"b":12345678901234567890,"c":[true,null,"x"]}"#
            );
        });
    }
}

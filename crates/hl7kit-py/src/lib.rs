//! PyO3 bindings for hl7kit. Thin by design: no logic here, only conversion.
//!
//! `Message` owns the message text, so byte spans stay valid for as long as
//! the Python object lives. Values come back unescaped.

use pyo3::create_exception;
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use std::collections::BTreeMap;

create_exception!(hl7kit, ParseError, PyValueError);

/// Parsed HL7 v2 message. Owns the text so spans stay valid.
#[pyclass(name = "Message", frozen)]
pub struct PyMessage {
    inner: hl7kit::Message,
}

impl PyMessage {
    /// The wrapped message, for other extension modules built on hl7kit.
    pub fn inner(&self) -> &hl7kit::Message {
        &self.inner
    }
}

#[pymethods]
impl PyMessage {
    /// MSH-9 as "ORM^O01", or None when MSH-9 is absent.
    #[getter]
    fn message_type(&self) -> Option<String> {
        self.inner
            .message_type()
            .map(|t| format!("{}^{}", t.code, t.trigger))
    }

    /// MSH-10.
    #[getter]
    fn control_id(&self) -> Option<String> {
        self.inner.control_id().map(str::to_string)
    }

    /// MSH-12.
    #[getter]
    fn version(&self) -> Option<String> {
        self.inner.version().map(str::to_string)
    }

    /// Unescaped value at an hl7kit path such as "PID-5", "OBR-18", "ZDS-1.1",
    /// "OBX[2]-5" or "PID-3[2].1". None when the element is absent; a
    /// ValueError for a path that is not well formed.
    fn get(&self, path: &str) -> PyResult<Option<String>> {
        hl7kit::Path::parse(path).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(self.inner.get_decoded(path).map(|v| v.into_owned()))
    }

    fn __getitem__(&self, path: &str) -> PyResult<String> {
        self.get(path)?
            .ok_or_else(|| PyKeyError::new_err(path.to_string()))
    }

    /// Byte span (start, end) of a path in the message text, for highlighting.
    fn span(&self, path: &str) -> PyResult<Option<(usize, usize)>> {
        hl7kit::Path::parse(path).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(self.inner.get_span(path).map(|s| (s.start, s.end)))
    }

    /// Segment names in order, e.g. ["MSH", "PID", "PV1", "ORC", "OBR", "ZDS"].
    fn segments(&self) -> Vec<String> {
        self.inner
            .segments()
            .map(|s| s.name().to_string())
            .collect()
    }

    /// Parser warnings (non-fatal irregularities in the message text).
    #[getter]
    fn warnings(&self) -> Vec<String> {
        self.inner
            .warnings()
            .iter()
            .map(|w| w.to_string())
            .collect()
    }

    /// Imaging-order linkage, or None when the message carries no order
    /// segment at all (no ORC, OBR or IPC).
    fn order(&self) -> Option<PyOrder> {
        let has_order = ["ORC", "OBR", "IPC"]
            .iter()
            .any(|s| self.inner.segment(s).is_some());
        has_order.then(|| PyOrder::from_message(&self.inner))
    }

    /// The message text as bytes (UTF-8).
    fn raw<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.raw().as_bytes())
    }

    fn __repr__(&self) -> String {
        format!(
            "<hl7kit.Message {} {} segments>",
            self.message_type().unwrap_or_else(|| "?".into()),
            self.inner.segment_count()
        )
    }
}

/// Study Instance UID with provenance.
#[pyclass(name = "StudyUid", frozen, get_all, skip_from_py_object)]
#[derive(Clone)]
pub struct PyStudyUid {
    pub value: String,
    /// "IPC-3.1" | "ZDS-1.1" | "OBX-110180"
    pub source: String,
}

/// The identifiers that link an order to its images (IHE RAD-4).
#[pyclass(name = "Order", frozen, get_all, skip_from_py_object)]
#[derive(Clone)]
pub struct PyOrder {
    /// PID-3.1
    pub patient_id: Option<String>,
    /// OBR-18 or IPC-1.1
    pub accession_number: Option<String>,
    /// OBR-19 or IPC-2.1
    pub requested_procedure_id: Option<String>,
    /// IPC-4.1 or OBR-20
    pub scheduled_procedure_step_id: Option<String>,
    pub study_uid: Option<PyStudyUid>,
    /// Human-readable warnings, e.g. a UID conflict between ZDS-1.1 and OBX.
    pub warnings: Vec<String>,
    /// Field name -> hl7kit path that supplied it.
    pub sources: BTreeMap<String, String>,
    /// Field name -> (start, end) byte span in the message text.
    pub spans: BTreeMap<String, (usize, usize)>,
}

fn source_name(s: hl7kit::order::StudyUidSource) -> &'static str {
    use hl7kit::order::StudyUidSource;
    match s {
        StudyUidSource::Ipc3 => "IPC-3.1",
        StudyUidSource::Zds1 => "ZDS-1.1",
        StudyUidSource::Obx => "OBX-110180",
    }
}

fn field_name(f: hl7kit::order::OrderField) -> &'static str {
    use hl7kit::order::OrderField;
    match f {
        OrderField::PatientId => "patient_id",
        OrderField::Accession => "accession_number",
        OrderField::ProcedureId => "requested_procedure_id",
        OrderField::StudyUid => "study_uid",
    }
}

impl PyOrder {
    fn from_message(msg: &hl7kit::Message) -> PyOrder {
        let o = hl7kit::order::Order::extract(msg);
        // hl7kit's Order stops at the four linkage identifiers; the SPS ID
        // (IHE: IPC-4, else OBR-20 Filler Field 1) is read here.
        let sps = msg
            .get_decoded("IPC-4.1")
            .or_else(|| msg.get_decoded("OBR-20"))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        PyOrder {
            patient_id: o.patient_id.clone(),
            accession_number: o.accession.clone(),
            requested_procedure_id: o.procedure_id.clone(),
            scheduled_procedure_step_id: sps,
            study_uid: match (&o.study_uid, o.study_uid_source) {
                (Some(v), Some(s)) => Some(PyStudyUid {
                    value: v.clone(),
                    source: source_name(s).to_string(),
                }),
                _ => None,
            },
            warnings: o
                .warnings
                .iter()
                .map(|w| describe_warning(msg, w))
                .collect(),
            sources: o
                .sources
                .iter()
                .map(|(f, p)| (field_name(*f).to_string(), p.to_string()))
                .collect(),
            spans: o
                .spans
                .iter()
                .map(|(s, f)| (field_name(*f).to_string(), (s.start, s.end)))
                .collect(),
        }
    }
}

#[pymethods]
impl PyOrder {
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("patient_id", &self.patient_id)?;
        d.set_item("accession_number", &self.accession_number)?;
        d.set_item("requested_procedure_id", &self.requested_procedure_id)?;
        d.set_item(
            "scheduled_procedure_step_id",
            &self.scheduled_procedure_step_id,
        )?;
        d.set_item(
            "study_uid",
            self.study_uid.as_ref().map(|u| u.value.clone()),
        )?;
        d.set_item(
            "study_uid_source",
            self.study_uid.as_ref().map(|u| u.source.clone()),
        )?;
        d.set_item("warnings", PyList::new(py, &self.warnings)?)?;
        d.set_item("sources", self.sources.clone())?;
        Ok(d)
    }

    fn __repr__(&self) -> String {
        format!(
            "<hl7kit.Order accession={:?} rp={:?} sps={:?} study_uid={:?} warnings={}>",
            self.accession_number,
            self.requested_procedure_id,
            self.scheduled_procedure_step_id,
            self.study_uid.as_ref().map(|u| u.value.as_str()),
            self.warnings.len()
        )
    }
}

/// hl7kit reports a UID conflict by byte spans; name the segments they fall
/// in, which is what a reader wants ("ZDS-1.1 used, OBX differs").
fn describe_warning(msg: &hl7kit::Message, w: &hl7kit::Warning) -> String {
    let segment_at = |offset: usize| -> String {
        msg.segments()
            .enumerate()
            .find(|(_, s)| {
                let sp = s.span();
                sp.start <= offset && offset < sp.end
            })
            .map(|(i, s)| {
                let name = s.name();
                let nth = msg.segments().take(i).filter(|t| t.name() == name).count() + 1;
                if nth > 1 {
                    format!("{name}[{nth}]")
                } else {
                    name.to_string()
                }
            })
            .unwrap_or_else(|| "?".to_string())
    };
    match w {
        hl7kit::Warning::ConflictingStudyUid { first, other } => format!(
            "{w} [{} used, {} differs]",
            segment_at(first.start),
            segment_at(other.start)
        ),
        #[allow(unreachable_patterns)]
        _ => w.to_string(),
    }
}

fn parse_bytes(bytes: &[u8]) -> Result<hl7kit::Message, hl7kit::ParseError> {
    // Latin-1 order messages are common in practice: fall back to a lossy
    // decode rather than refuse the file.
    match hl7kit::Message::parse_bytes(bytes) {
        Err(hl7kit::ParseError::InvalidUtf8 { .. }) => hl7kit::Message::parse_lossy(bytes),
        other => other,
    }
}

/// Parse one message from bytes or str (ER7; CR, LF or CRLF segment terminators).
#[pyfunction]
fn parse(py: Python<'_>, data: Bound<'_, PyAny>) -> PyResult<PyMessage> {
    let bytes: Vec<u8> = if let Ok(b) = data.cast::<PyBytes>() {
        b.as_bytes().to_vec()
    } else {
        data.extract::<String>()?.into_bytes()
    };
    // Without the GIL: a batch job can parse in threads.
    let inner = py
        .detach(move || parse_bytes(&bytes))
        .map_err(|e| ParseError::new_err(e.to_string()))?;
    Ok(PyMessage { inner })
}

/// Parse a batch (list of bytes) with the GIL released; returns a list of Message.
#[pyfunction]
fn parse_many(py: Python<'_>, items: Vec<Vec<u8>>) -> PyResult<Vec<PyMessage>> {
    let parsed: Result<Vec<_>, _> =
        py.detach(move || items.iter().map(|b| parse_bytes(b)).collect());
    parsed
        .map(|v| v.into_iter().map(|inner| PyMessage { inner }).collect())
        .map_err(|e| ParseError::new_err(e.to_string()))
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMessage>()?;
    m.add_class::<PyOrder>()?;
    m.add_class::<PyStudyUid>()?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(parse_many, m)?)?;
    m.add("ParseError", m.py().get_type::<ParseError>())?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

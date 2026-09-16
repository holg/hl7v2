//! PyO3 bindings for mwlkit. Output is bytes (a Part 10 file, Explicit VR
//! Little Endian), so pydicom users do `pydicom.dcmread(BytesIO(item.to_bytes()))`.
//! No Rust data set type is exposed to Python: pydicom owns that role.
//!
//! `worklist_item` accepts an `hl7kit.Message` (duck-typed through its
//! `raw()` method), bytes, or str. A second extension module cannot share
//! the first one's Python type object, so the message is re-parsed here;
//! that costs microseconds and keeps the two wheels independent.

use dicom_core::header::Header;
use dicom_core::value::Value;
use dicom_core::DataDictionary;
use dicom_dictionary_std::StandardDataDictionary;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

/// A Modality Worklist item built from one HL7 order.
#[pyclass(name = "WorklistItem", frozen)]
pub struct PyWorklistItem {
    part10: Vec<u8>,
    study_uid: String,
    study_uid_origin: String,
    warnings: Vec<String>,
    dataset: dicom_object::InMemDicomObject,
}

#[pymethods]
impl PyWorklistItem {
    /// The item as a DICOM Part 10 file (128-byte preamble, DICM, file meta
    /// with the Modality Worklist FIND SOP Class, Explicit VR Little Endian).
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.part10)
    }

    /// (0020,000D) as written.
    #[getter]
    fn study_uid(&self) -> &str {
        &self.study_uid
    }

    /// Where the UID came from: "IPC-3.1", "ZDS-1.1", "OBX-110180", or
    /// "generated:RequestedProcedureId" | "generated:AccessionNumber" |
    /// "generated:Random".
    #[getter]
    fn study_uid_origin(&self) -> &str {
        &self.study_uid_origin
    }

    /// Every decision the mapping made on its own, as text.
    #[getter]
    fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }

    /// The data set as nested dicts: "(gggg,eeee)" -> str value, or a list
    /// of dicts for sequences.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        dataset_to_dict(py, &self.dataset)
    }

    /// Like `to_dict`, keyed by dictionary keyword ("AccessionNumber")
    /// where one exists, else by tag.
    fn to_keyword_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        dataset_to_keyword_dict(py, &self.dataset)
    }

    fn __repr__(&self) -> String {
        format!(
            "<mwlkit.WorklistItem study_uid={} ({}) {} bytes, {} warning(s)>",
            self.study_uid,
            self.study_uid_origin,
            self.part10.len(),
            self.warnings.len()
        )
    }
}

fn element_value<'py>(
    py: Python<'py>,
    e: &dicom_object::mem::InMemElement,
) -> PyResult<Bound<'py, PyAny>> {
    match e.value() {
        Value::Sequence(seq) => {
            let list = PyList::empty(py);
            for item in seq.items() {
                list.append(dataset_to_dict(py, item)?)?;
            }
            Ok(list.into_any())
        }
        Value::PixelSequence(_) => Ok(PyList::empty(py).into_any()),
        Value::Primitive(p) => Ok(p.to_str().trim().to_string().into_pyobject(py)?.into_any()),
    }
}

fn dataset_to_dict<'py>(
    py: Python<'py>,
    ds: &dicom_object::InMemDicomObject,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for e in ds.iter() {
        d.set_item(e.tag().to_string(), element_value(py, e)?)?;
    }
    Ok(d)
}

fn dataset_to_keyword_dict<'py>(
    py: Python<'py>,
    ds: &dicom_object::InMemDicomObject,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for e in ds.iter() {
        let key = StandardDataDictionary
            .by_tag(e.tag())
            .map(|entry| entry.alias.to_string())
            .unwrap_or_else(|| e.tag().to_string());
        let value = match e.value() {
            Value::Sequence(seq) => {
                let list = PyList::empty(py);
                for item in seq.items() {
                    list.append(dataset_to_keyword_dict(py, item)?)?;
                }
                list.into_any()
            }
            _ => element_value(py, e)?,
        };
        d.set_item(key, value)?;
    }
    Ok(d)
}

fn message_text(message: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(b) = message.cast::<PyBytes>() {
        return Ok(b.as_bytes().to_vec());
    }
    if let Ok(s) = message.extract::<String>() {
        return Ok(s.into_bytes());
    }
    // hl7kit.Message: its raw() gives the text back.
    if let Ok(raw) = message.call_method0("raw") {
        if let Ok(b) = raw.cast::<PyBytes>() {
            return Ok(b.as_bytes().to_vec());
        }
    }
    Err(PyTypeError::new_err(
        "message must be an hl7kit.Message, bytes or str",
    ))
}

fn origin_name(o: mwlkit::StudyUidOrigin) -> String {
    use hl7kit::order::StudyUidSource;
    match o {
        mwlkit::StudyUidOrigin::FromOrder(StudyUidSource::Ipc3) => "IPC-3.1".into(),
        mwlkit::StudyUidOrigin::FromOrder(StudyUidSource::Zds1) => "ZDS-1.1".into(),
        mwlkit::StudyUidOrigin::FromOrder(StudyUidSource::Obx) => "OBX-110180".into(),
        mwlkit::StudyUidOrigin::Generated(g) => format!("generated:{g:?}"),
    }
}

/// worklist_item(message, uid_policy="dcm4che", length_policy="refuse",
///               default_station_ae=None, charset=None)
///
/// uid_policy: "dcm4che" derives a name-based UID from the Requested
/// Procedure ID or the Accession Number when the order has none; "random"
/// uses 128 bits from os.urandom; "refuse" raises. length_policy: "refuse"
/// raises on SH/LO/PN values over the VR limit, "truncate" cuts and warns.
/// charset: a DICOM term for (0008,0005), else MSH-18 is read.
#[pyfunction]
#[pyo3(signature = (message, uid_policy="dcm4che", length_policy="refuse", default_station_ae=None, charset=None))]
fn worklist_item(
    py: Python<'_>,
    message: Bound<'_, PyAny>,
    uid_policy: &str,
    length_policy: &str,
    default_station_ae: Option<String>,
    charset: Option<String>,
) -> PyResult<PyWorklistItem> {
    let policy = match uid_policy {
        "dcm4che" => mwlkit::UidPolicy::Dcm4cheStyle,
        "random" => {
            let bytes: Vec<u8> = py.import("os")?.call_method1("urandom", (16,))?.extract()?;
            let mut bits = [0u8; 16];
            bits.copy_from_slice(&bytes[..16]);
            mwlkit::UidPolicy::Random(u128::from_be_bytes(bits))
        }
        "refuse" => mwlkit::UidPolicy::Refuse,
        other => {
            return Err(PyValueError::new_err(format!(
                "uid_policy: {other:?} (dcm4che, random or refuse)"
            )))
        }
    };
    let length = match length_policy {
        "refuse" => mwlkit::LengthPolicy::Refuse,
        "truncate" => mwlkit::LengthPolicy::TruncateAndWarn,
        other => {
            return Err(PyValueError::new_err(format!(
                "length_policy: {other:?} (refuse or truncate)"
            )))
        }
    };
    let text = message_text(&message)?;
    let msg = match hl7kit::Message::parse_bytes(&text) {
        Err(hl7kit::ParseError::InvalidUtf8 { .. }) => hl7kit::Message::parse_lossy(&text),
        other => other,
    }
    .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let order = hl7kit::order::Order::extract(&msg);
    let opts = mwlkit::Options {
        uid_policy: policy,
        length_policy: length,
        default_station_ae,
        character_set: charset,
    };
    let item = mwlkit::worklist_item(
        &mwlkit::Input {
            message: &msg,
            order: &order,
        },
        &opts,
    )
    .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let part10 = mwlkit::to_bytes(&item).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyWorklistItem {
        part10,
        study_uid: item.study_uid.value.clone(),
        study_uid_origin: origin_name(item.study_uid.origin),
        warnings: item.warnings.iter().map(|w| w.to_string()).collect(),
        dataset: item.dataset,
    })
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyWorklistItem>()?;
    m.add_function(wrap_pyfunction!(worklist_item, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

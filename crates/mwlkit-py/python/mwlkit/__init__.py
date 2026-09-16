"""mwlkit: HL7 v2 order -> DICOM Modality Worklist item, bytes out, pydicom in."""
from io import BytesIO
from ._native import WorklistItem, worklist_item, __version__

__all__ = ["WorklistItem", "worklist_item", "to_pydicom", "__version__"]


def to_pydicom(item: WorklistItem):
    """Return a pydicom Dataset. pydicom is an optional dependency."""
    import pydicom  # local import: keeps the base package dependency-free
    return pydicom.dcmread(BytesIO(item.to_bytes()))

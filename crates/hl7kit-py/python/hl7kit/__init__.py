"""hl7kit: HL7 v2.x parsing with byte spans and imaging-order linkage.

    >>> import hl7kit
    >>> msg = hl7kit.parse(open("order.hl7", "rb").read())
    >>> msg["PID-5"], msg.get("OBR-18")
    >>> o = msg.order(); o.study_uid.value, o.study_uid.source, o.warnings
"""
from ._native import Message, Order, StudyUid, ParseError, parse, parse_many, __version__

__all__ = ["Message", "Order", "StudyUid", "ParseError", "parse", "parse_many", "__version__"]

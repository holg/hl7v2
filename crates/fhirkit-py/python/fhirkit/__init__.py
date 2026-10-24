"""fhirkit: FHIR R4 in Rust. Every resource and datatype from the official
specification, lossless JSON, and validation that reports every problem with
its path.

    >>> import fhirkit
    >>> r = fhirkit.parse('{"resourceType": "Patient", "birthDate": "1974-12-25"}')
    >>> r.resource_type, r.issues
    ('Patient', [])
    >>> [str(i) for i in fhirkit.validate({"resourceType": "Patient", "gender": "x y "})]
    ['error at Patient.gender: "x y " is not a valid code']
"""
from ._native import (
    FHIR_VERSION,
    RESOURCE_TYPES,
    Issue,
    ParseError,
    Resource,
    ValidationError,
    __version__,
    parse,
    parse_many,
    validate,
)

__all__ = [
    "FHIR_VERSION",
    "RESOURCE_TYPES",
    "Issue",
    "ParseError",
    "Resource",
    "ValidationError",
    "__version__",
    "parse",
    "parse_many",
    "validate",
]

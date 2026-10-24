from typing import Any, Union

__version__: str
FHIR_VERSION: str
RESOURCE_TYPES: list[str]

class ParseError(ValueError): ...

class ValidationError(ValueError):
    issues: list[Issue]

class Issue:
    severity: str  # "error" | "warning"
    path: str  # "Patient.name[0].given[1]"
    message: str

class Resource:
    @property
    def resource_type(self) -> str: ...
    @property
    def id(self) -> str | None: ...
    @property
    def issues(self) -> list[Issue]: ...
    def to_json(self, pretty: bool = False) -> str: ...
    def to_dict(self) -> dict[str, Any]: ...

def parse(data: Union[str, bytes, dict[str, Any]], strict: bool = False) -> Resource: ...
def parse_many(
    items: list[Union[str, bytes, dict[str, Any]]], threads: int = 0, strict: bool = False
) -> list[Resource]: ...
def validate(data: Union[str, bytes, dict[str, Any]]) -> list[Issue]: ...

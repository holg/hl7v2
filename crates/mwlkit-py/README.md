# mwlkit (Python)

```python
import hl7kit, mwlkit
item = mwlkit.worklist_item(hl7kit.parse(open("order.hl7","rb").read()), default_station_ae="CT1_AE")
ds = mwlkit.to_pydicom(item)     # pydicom Dataset, or item.to_bytes() for Orthanc/dcm4chee tooling
item.study_uid, item.study_uid_origin, item.warnings
```

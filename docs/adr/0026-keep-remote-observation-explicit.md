# Keep Remote Observation Explicit

Fetch and `list --remote` are Taku's explicit broad-read workflows. Status and Diff never contact a Target, and Pull requires a structurally valid Observed State Cache. Push performs only reads required by selected Operations, existence guarantees, or concurrency guards and never hides an implicit Many Fetch inside planning or execution.

Observed State does not expire solely with elapsed time. Taku binds it to the exact Project inputs, installed Application definitions, Target, discovered Application Version, and selected Resource Type Definitions and prominently reports its age; declared concurrency protection, rather than an arbitrary TTL, determines write safety. After a successful mutation, Taku updates the affected cached observation only when the Operation declares its response trustworthy and transformable into Observed State. Otherwise it invalidates that entry until Fetch, while the Push Journal independently prevents duplicate execution.

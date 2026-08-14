# Reject unknown configuration

Every Taku-owned file format will carry an explicit schema version, and validation will reject unknown fields, invalid references, ambiguous Variants, unsupported Operations, Transformation errors, and dependency cycles before any network call. Future `taku migrate` operations will change local files only and leave reviewable Git diffs; this favors typo detection and predictable execution over silently accepting configuration intended for a newer or different schema.

# Keep Resource Metadata Policy Directory-Local

Application catalogs classify API-owned Resource Metadata, but each repository chooses whether to track it through closed `.target.yaml` and `.resource.yaml` Directory Hints whose identity comes from their physical location. This preserves self-contained Target and Resource Type directories and avoids duplicating the desired-state hierarchy in `.taku/project.yaml`; the trade-off is that broad policy must be repeated once per Target, while Promotion deliberately keeps destination policy authoritative instead of copying source hints.

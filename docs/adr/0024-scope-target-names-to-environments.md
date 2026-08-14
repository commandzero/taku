# Scope Target Names to Environments

Target names are Environment-specific rather than Project-wide because corresponding remote instances commonly have different operational names, such as `dev/es-dev` and `prod/es-prod`. `taku app rename <old> <new>` changes only the current or explicitly selected Environment's Target configuration and Resource tree, preserves Resource IDs, invalidates Target-bound caches and incomplete Push Journals, performs no remote calls, and leaves Git changes unstaged.

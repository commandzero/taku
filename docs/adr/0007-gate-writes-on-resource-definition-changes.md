# Gate writes on Resource Type Definition changes

Each Environment and Target will keep a non-secret Target Baseline recording the discovered Application Version, Major Version Catalog, and Resource Type Definitions against which its Canonical Representations were reconciled. A version change that selects different definitions blocks Push until Fetch and Pull regenerate compatible Resources and update the baseline; Taku does not upgrade or downgrade applications, and reverting Git state does not promise compatibility with a newer Target.

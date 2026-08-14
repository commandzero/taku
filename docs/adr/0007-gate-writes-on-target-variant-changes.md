# Gate writes on Target Variant changes

Each Environment and Target will keep a non-secret Target Baseline recording the facts and Resource Type Variants against which its Canonical Representations were reconciled. Fetch reports fact changes that remain compatible, but a change selecting different Variants blocks Push until Pull regenerates compatible Resources and updates the baseline; Taku does not upgrade or downgrade applications, and reverting Git state does not promise compatibility with a newer Target.

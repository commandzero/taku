# Use three-way comparison for Pull

Fetch will record the canonical hash of each selected working-tree Resource alongside its Observed State, allowing Pull to distinguish local-only changes, remote-only changes, equal changes, and divergent changes. Divergence becomes a Pull Conflict without markers or file mutation, while safe changes are staged and applied atomically where the filesystem permits; Git remains responsible for reviewing and reverting the resulting working-tree diff.

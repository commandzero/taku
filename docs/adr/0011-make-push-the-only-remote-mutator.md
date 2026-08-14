# Make Push the only remote mutator

Push will be the only Taku workflow that changes remote application state; Add, Pull, Promotion, Remove, Forget, and migration prepare local desired state, while Fetch, local or remote List, Status, Diff, and validation remain read-only. In particular, Remove creates a guarded Deletion Marker for a later Push and Taku will not offer a direct-delete shortcut, preserving a reviewable Git-tracked statement of every intended remote change.

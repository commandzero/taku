# Journal Partial Push Execution

Before its first remote mutation, Push will durably record an ignored Push Journal bound to the exact plan, selected input hashes, installed Application definitions, Target Facts, and Target. It records each confirmed outcome as execution proceeds. A retry with an identical binding resumes unfinished work and does not repeat confirmed successes, including successful writes whose remote system increments a version on every invocation.

Any relevant input or binding change prevents resumption and requires an explicit new plan rather than guessing which results remain valid. A completed Push Journal becomes a disposable execution report. The journal supports recovery and audit of one execution attempt; it is neither desired state nor a rollback mechanism.

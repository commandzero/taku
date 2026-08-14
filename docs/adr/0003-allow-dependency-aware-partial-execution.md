# Allow dependency-aware partial execution

Taku will validate selected Resources locally before mutation, but it will not promise transaction-wide remote preflight, atomic execution, or rollback across arbitrary APIs. Each operation declares its Retry Safety; after safe retries are exhausted, a failure blocks dependent Resources while independent work continues, and the command returns a non-successful per-Resource execution report so a later run can reconcile what remains.

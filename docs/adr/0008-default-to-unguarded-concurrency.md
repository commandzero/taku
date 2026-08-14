# Default to Unguarded concurrency

Resource Types will default to Unguarded mutation so Taku can manage APIs without conditional-write support, while allowing stronger concurrency behavior to be configured and inherited by their Operations. Taku may re-read immediately before an Unguarded write and must expose the remaining race in its plan, but it will not imply that this best-effort check is atomic or let the Unguarded default weaken an Operation's separately declared Retry Safety.

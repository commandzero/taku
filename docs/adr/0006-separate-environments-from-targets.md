# Separate Environments from Targets

An Environment will represent a deployment and Promotion boundary, while each Target represents one remote application endpoint within that Environment; Resource Types are assigned to Targets. This additional layer allows one Environment to manage related systems such as Elasticsearch and Kibana together without treating their URLs, credentials, versions, or application-specific behavior as separate promotion domains.

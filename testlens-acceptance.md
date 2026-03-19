High overview of acceptance tests for testlens (different stages)

Stage - 1
Description: we can discover all possible type of test artefacts, and using call-sites relate them to existing (production) artefacts

GIven an initialized repository with production artefacts
When i ingest-tests
Then test artefacts and relationships to production artefacts are created, and can be queried

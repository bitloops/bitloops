# bitloops-engine

Workflow Orchestration Engine for Bitloops

## System Setup

To setup your system load the following docker containers:

`docker network create bitloops`

**NATS**

`docker pull nats:latest`  
`docker run -p 4222:4222 -p 8222:8222 -ti --name nats --network bitloops nats:latest`  
Note that we are exposing two ports. 4222 for the server itself and 8222 for the monitoring service.

**NATS Web UI (optional)**

`docker pull sphqxe/nats-webui:latest`  
`docker run -p 8082:80 --name nats-webui --network bitloops -ti sphqxe/nats-webui:latest`  
You can visit the UI here: [http://localhost:8082/]  
(check the docker container IP from the logs and add this as the NATS server IP)

**Redis**

`docker pull redis:latest`  
`docker run -p 6379:6379 -ti --name redis --network bitloops redis:latest`

**Mongo**

`docker pull mongo:latest`  
`docker run -p 27017:27017 -ti --name mongo --network bitloops mongo:latest`

## Running the server

`yarn`  
`yarn start`

## Using Mutliple Docker Containers on the same machine

`docker network create bitloops`  
`docker network connect bitloops nats`  
`docker network connect bitloops redis`  
`docker network connect bitloops nats-webui`  
`docker network connect bitloops mongo`  
etc...

To get a containers IP run:  
`docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' nats`
172.17.0.2

# Required ENV variables

NATS Options:

NATS_IP  
NATS_PORT  
NATS_USER  
NATS_PASSWORD

\*To load local NATS remove the above variables

NATS Topics:

ENGINE_NATS_TOPIC  
ENGINE_ADMIN_NATS_TOPIC  
ENGINE_EVENTS_NATS_TOPIC  
ENGINE_GRPC_NATS_TOPIC  
ENGINE_LOGGER_NATS_TOPIC

NATS Queue:

BITLOOPS_ENGINE_QUEUE

Redis:

REDIS_HOST  
REDIS_PORT  
REDIS_PASSWORD

\*To load local Redis remove the above variables

BigQuery:

GCP_CREDENTIALS_JSON  
BIGQUERY_PROJECT_ID

Mongo:  
MONGO_URL

# Populate Mongo with test data

-   API credentials
-   Message to Workflows mappings
-   Workspaces
-   Users
-   Projects
-   Secrets
-   Workflows

## Build and deploy to AWS

- Build the image:
In `package.json` script: `build-push-image` upgrade the tag to the new version and then run `yarn build-push-image`.
- Deploy service to AWS:
In `charts/bitloops-engine/values.prod.yaml` upgrade the `image.tag` to the new version and then run `yarn aws-deploy-prod`.

- Build and Deploy to AWS:
In `package.json` script: `aws-build-deploy-prod` upgrade the tag to the new version,
in `charts/bitloops-engine/values.prod.yaml` upgrade the `image.tag` to the new version and then run `yarn aws-build-deploy-prod`.

## License

The Bitloops Engine is [fair-code](http://faircode.io) distributed under [**Apache 2.0 with Commons Clause**](https://github.com/bitloops/bitloops-engine/LICENSE.md) license.
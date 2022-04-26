# bitloops-rest

Bitloops REST API Bridge

## Running

`yarn`
`yarn start:dev`

Visit [http://localhost:8080/ping]

To create your Google Cloud build run:
gcloud builds submit --tag gcr.io/bitloops-managed/bitloops-rest

To deploy run the following or deploy through the console (https://console.cloud.google.com/run/deploy/europe-west1/bitloops-rest?project=bitloops-managed):
gcloud run deploy --image gcr.io/bitloops-managed/bitloops-rest --platform managed

You can also run:
yarn gcloud-build
yarn deploy

## System Setup

To setup your system load the following docker containers:

**Jaeger**

```bash
docker run -d --name jaeger \
  -e COLLECTOR_ZIPKIN_HOST_PORT=:9411 \
  -p 5775:5775/udp \
  -p 6831:6831/udp \
  -p 6832:6832/udp \
  -p 5778:5778 \
  -p 16686:16686 \
  -p 14268:14268 \
  -p 14250:14250 \
  -p 9411:9411 \
  jaegertracing/all-in-one:latest
```

**Prometheus**

```
docker run -d \
    -p 9090:9090 \
    -v ${PWD}/prometheus.yml:/etc/prometheus/prometheus.yml \
    --name prometheus prom/prometheus

```

## Deploy to Kubernetes

### Build Image

```bash
docker build -t bitloops/rest:$TAG .
```

### Push Image

```bash
docker push bitloops/rest:$TAG
```

### Deploy

Change TAG in values.$ENV.yaml

Navigate to charts folder:

```bash
helm upgrade --install bitloops-rest bitloops-rest --values=bitloops-rest/values.prod.yaml
```

## Local docker

```
docker build -t bitloops/rest . -f Dockerfile-local
```

```
docker run -d -p 3005:3005 -ti --name bitloops-rest --network bitloops bitloops/rest
```

## License

The Bitloops REST is [fair-code](http://faircode.io) distributed under [**Apache 2.0 with Commons Clause**](https://github.com/bitloops/bitloops-engine/LICENSE.md) license.

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
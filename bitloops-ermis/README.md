# bitloops-ermis
Bitloops Ermis for Subscriptions

## Running

`yarn`
`yarn start:dev`

## Local docker
```
docker build -t bitloops/ermis . -f Dockerfile-local
```
```
docker run -d -p 3006:3006 -ti --name bitloops-ermis --network bitloops_default bitloops/ermis
```
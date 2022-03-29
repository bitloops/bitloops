```bash
docker-compose up -d

# or to rebuild all images
docker-compose up -d --build
```

Use environmentId: `development` which is pointing to your csrv-mongo docker instance

Fetch all workflows of workspace

```bash
curl --location --request POST 'http://localhost:3005/bitloops/request' \
--header 'workflow-id: 7dcd2115-4536-48a4-97c0-26cea7f21768' \
--header 'node-id: c2115d23-f4da-46bb-aa86-0af8397cc4c7' \
--header 'environment-id: development' \
--header 'workflow-version: v1' \
--header 'authorization: anonymous' \
--header 'Content-Type: application/json' \
--data-raw '{
    "workspaceId": "db24bb48-d2e3-4433-8fd0-79eef2bf63df"
}'
```

To start an HTTP tunnel forwarding your bitloops-rest

Follow [ngrok setup](https://dashboard.ngrok.com/get-started/setup), run this next

```bash
ngrok http 3005
```

# Configuring Keycloak

```bash
sh ./scripts/install.sh
```

Enter your provider Id which is your realm name and the clint id which is your client name

## Add google as identity provider for Keycloak

- You need to fill in your client id and client secret from GCP(https://console.cloud.google.com/apis/credentials), OAuth 2.0 Clients
- Copy Redirect URI from keycloak(1st field) into Authorised redirect URIs in gcp(for your oauth2 client)

### How to add image from Google in claims

Under Identity Providers -> google -> Mappers tab -> Create

- Name = Picture
- Mapper Type = Attribute Importer
- Social Profile JSON Field Path = picture
- User Attribute Name = photoURL

Under Clients -> <Your_Client> -> Mappers tab -> Create

- Name = Picture
- Mapper Type = User Attribute
- User Attribute = photoURL
- Token Claim Name = photoURL
- Add to ID token = enabled

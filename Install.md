```bash
docker compose up -d

# or to rebuild all images
docker compose up -d --build
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

Visit http://localhost:8080

- Administration Console
- Login (admin, Pa55w0rd)
- From top left corner hoven on Master
- Create a realm, give a name (this will be your providerId)

## Configure your client inside Keycloak

- Navigate to **Clients** from left SidePanel
- Create, provide a name as Client ID (this will be your clientId)
- Save

### Go to new client’s **settings**

<!-- - Access Type-> Confidential
- Service Accounts Enabled
- Authorization Enabled -->

- Under Valid Redirect URIs add
  - http://localhost/bitloops/auth/google/final-callback
  - http://localhost/bitloops/auth/github/final-callback
- Save
  <!-- We will use client ID and client Secret(From Credentials Tab of client) -->

## Add google as identity provider for Keycloak

- Navigate to Identity providers
- Add provider.. -> google
- You need to fill in your client id and client secret from GCP(https://console.cloud.google.com/apis/credentials), OAuth 2.0 Clients
- Copy Redirect URI from keycloak(1st field) into Authorised redirect URIs in gcp(for your oauth2 client)

- Save

## Environment Variables for engine & rest

| name        | desc                                                                                                                                                          |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| KEYCLOAK_PK | Obtain Public key from <KEYCLOAK_URI>/auth/realms/{realm}, <br/>Base64 Encode it <br/>`-----BEGIN PUBLIC KEY-----` <br/>`...`<br/>`-----END PUBLIC KEY----- ` |

## Getting KeyCloak realm's public key

- Going to http://localhost:8080/auth/realms/bitloops (host is keycloak server)

- Add -----BEGIN PUBLIC KEY----- and append -----END PUBLIC KEY----- to this copied public key to use it anywhere to verify the JWTtoken. You public key should finally look something like this:

```nodejs
-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAhAj9OCZd0XjzOIad2VbUPSMoVK1X8hdD2Ad+jUXCzhZJf0RaN6B+79AW5jSgceAgyAtLXiBayLlaqSjZM6oyti9gc2M2BXzoDKLye+Tgpftd72Zreb4HpwKGpVrJ3H3Ip5DNLSD4a1ovAJ6Sahjb8z34T8c1OCnf5j70Y7i9t3y/j076XIUU4vWpAhI9LRAOkSLqDUE5L/ZdPmwTgK91Dy1fxUQ4d02Ly4MTwV2+4OaEHhIfDSvakLBeg4jLGOSxLY0y38DocYzMXe0exJXkLxqHKMznpgGrbps0TPfSK0c3q2PxQLczCD3n63HxbN8U9FPyGeMrz59PPpkwIDAQAB
-----END PUBLIC KEY-----
```

- [Base64 encode](https://www.base64encode.org/)

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

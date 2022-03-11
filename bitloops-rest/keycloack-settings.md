## Create a client credentials

From top left corner create a realm (e.g. bitloops)

## Configure your client inside Keycloak

Any frontend application that wants to access data

- Navigate to **clients page**
- Create
- provide only a name for client-id
- Save

### Go to new client’s **settings**

- Access Type-> Confidential
- Service Accounts Enabled
- Authorization Enabled

<!-- Finally provide a Redirect URI (uri of frontend app) and Save   -->

- Provide rest endpoint as Redirect URI  
  format: <REST_URI>/{realm}/auth/google/final-callback
  (e.g. http://127.0.0.1:3005/bitloops/auth/google/final-callback), rest server will have to match equivalent env-var of bitloops-rest
- Save
  <!-- We will use client ID and client Secret(From Credentials Tab of client) -->

## Add google as identity provider for Keycloak

- Navigate to Identity providers
- Add provider.. -> google
- You need to fill in client id and client secret from GCP(https://console.cloud.google.com/apis/credentials), OAuth 2.0 Clients
- Add `<Keycloak-server>/auth/realms/bitloops/broker/google/endpoint` endpoint to Authorised redirect URIs

- Save

## Environment Variables

| name                   | desc                                                                                                                                                          |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| KEYCLOAK_CLIENT_SECRET | Client -> Credentials Tab                                                                                                                                     |
| KEYCLOAK_URI           | e.g. http://localhost:8080                                                                                                                                    |
| REST_URI               | e.g. http://localhost:3005                                                                                                                                    |
| KEYCLOAK_PK            | Obtain Public key from <KEYCLOAK_URI>/auth/realms/{realm}, <br/>Base64 Encode it <br/>`-----BEGIN PUBLIC KEY-----` <br/>`...`<br/>`-----END PUBLIC KEY----- ` |
| COOKIES_DOMAIN         | localhost                                                                                                                                                     |

## Getting the public key of the KeyCloak server

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

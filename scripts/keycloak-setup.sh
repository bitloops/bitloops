#!/bin/bash
shopt -s lastpipe
echo fetching token id
curl --location --request POST 'http://localhost:8080/auth/realms/master/protocol/openid-connect/token' \
--header 'Content-Type: application/x-www-form-urlencoded' \
--data-urlencode 'grant_type=password' \
--data-urlencode 'client_id=admin-cli' \
--data-urlencode 'username=admin' \
--data-urlencode 'password=Pa55w0rd' | jq -r '.access_token' | read ACESS_KEY
echo creating realm
GOOGLE_URI="http://127.0.0.1/$1/auth/google/final-callback"
GITHUB_URI="http://127.0.0.1/$1/auth/github/final-callback"

curl -X POST "http://localhost:8080/auth/admin/realms"\
  -H "Content-Type: application/json"\
  -H "Authorization: bearer $ACESS_KEY"\
  -d '
{
    "realm": "'"$1"'",
    "enabled": true,
    "users": [
        {
            "username": "keycloak",
            "enabled": true,
            "credentials": [ {
                    "type": "password",
                    "value": "test"
                }
            ],
            "realmRoles": [
                "user"
            ]
        }
    ],
    "roles": {
        "realm": [
            {
                "name": "user",
                "description": "User privileges"
            },
            {
                "name": "admin",
                "description": "Administrator privileges"
            }
        ]
    },
    "defaultRoles": [
        "user"
    ],
    "clients": [
        {
            "clientId": "'"$2"'",
            "enabled": true,
            "publicClient": true,
            "redirectUris" : [
            "'"$GOOGLE_URI"'",
            "'"$GITHUB_URI"'"
            ],
            "webOrigins": [
                "*"
            ]
        }
    ]
}
'
curl -X POST "http://localhost:8080/auth/admin/realms/$1/identity-provider/instances"\
  -H "Content-Type: application/json"\
  -H "Authorization: bearer $ACESS_KEY"\
  -d '{
    "alias": "google",
    "providerId": "google",
    "config": {
    "clientId":"'"$3"'",
    "clientSecret":"'"$4"'",
    "syncMode":"IMPORT"
  }
}'
echo your redirect URI is : "http://localhost:8080/auth/realms/$1/broker/google/endpoint"
echo '-----BEGIN PUBLIC KEY-----' > .public_key.txt;
curl http://localhost:8080/auth/realms/"$1" | jq '.public_key' | xargs echo  >> .public_key.txt;
echo '-----END PUBLIC KEY-----' >> .public_key.txt
base64 ./.public_key.txt | xargs echo > .public_key_base64.txt
PK=$(cat ./.public_key_base64.txt)
sed -i.bak "s/^KEYCLOAK_PK.*$/KEYCLOAK_PK='$PK'/g" ../bitloops-rest/docker.env 
sed -i.bak "s/^KEYCLOAK_PK.*$/KEYCLOAK_PK='$PK'/g" ../bitloops-engine/docker.env 

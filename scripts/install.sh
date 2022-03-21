#!/bin/bash
STATUS_CODE=000
docker-compose up -d keycloak
while [ $STATUS_CODE == 000 ] 
do 
  STATUS_CODE=$( curl --write-out '%{http_code}\n' --silent --output /dev/null http://localhost:8080/auth/realms/master/protocol/openid-connect/token)
  echo connecting to keycloak ... 
  sleep 5
  done
shopt -s lastpipe
echo fetching token id
curl --location --request POST 'http://localhost:8080/auth/realms/master/protocol/openid-connect/token' \
--header 'Content-Type: application/x-www-form-urlencoded' \
--data-urlencode 'grant_type=password' \
--data-urlencode 'client_id=admin-cli' \
--data-urlencode 'username=admin' \
--data-urlencode 'password=Pa55w0rd' | jq -r '.access_token' | read ACESS_KEY&&
echo insert your provider id &&
read PID

echo insert your client id &&
read CID
GOOGLE_URI="http://127.0.0.1/$PID/auth/google/final-callback"
GITHUB_URI="http://127.0.0.1/$PID/auth/github/final-callback"

curl -X POST "http://localhost:8080/auth/admin/realms"\
  -H "Content-Type: application/json"\
  -H "Authorization: bearer $ACESS_KEY"\
  -d '
{
    "realm": "'"$PID"'",
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
            "clientId": "'"$CID"'",
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
echo your redirect URI is : "http://localhost:8080/auth/realms/$PID/broker/google/endpoint"

echo insert your Client Id from GCP&&
read GCID

echo insert your Client Secret from GCP&&
read CS
curl -X POST "http://localhost:8080/auth/admin/realms/$PID/identity-provider/instances"\
  -H "Content-Type: application/json"\
  -H "Authorization: bearer $ACESS_KEY"\
  -d '{
    "alias": "google",
    "providerId": "google",
    "config": {
    "clientId":"'"$GCID"'",
    "clientSecret":"'"$CS"'",
    "syncMode":"IMPORT"
  }
}'
echo '-----BEGIN PUBLIC KEY-----' > .public_key.txt;
curl http://localhost:8080/auth/realms/"$PID" | jq '.public_key' | xargs echo  >> .public_key.txt;
echo '-----END PUBLIC KEY-----' >> .public_key.txt
base64 ./.public_key.txt | xargs echo > .public_key_base64.txt
PK=$(cat ./.public_key_base64.txt)
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
PARENT="$(dirname "$SCRIPT_DIR")"
sed -i.bak "s/^KEYCLOAK_PK.*$/KEYCLOAK_PK='$PK'/g" $PARENT/bitloops-rest/docker.env
sed -i.bak "s/^KEYCLOAK_PK.*$/KEYCLOAK_PK='$PK'/g" $PARENT/bitloops-engine/docker.env
docker-compose up -d --scale keycloak=0

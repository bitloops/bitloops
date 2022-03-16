shopt -s lastpipe
echo fetching token id
curl --location --request POST 'http://localhost:8080/auth/realms/master/protocol/openid-connect/token' \
--header 'Content-Type: application/x-www-form-urlencoded' \
--data-urlencode 'grant_type=password' \
--data-urlencode 'client_id=admin-cli' \
--data-urlencode 'username=admin' \
--data-urlencode 'password=Pa55w0rd' | jq -r '.access_token' | read ACESS_KEY
echo creating realm
curl --location -X POST "http://localhost:8080/auth/admin/realms/"\
  -H "Content-Type: application/json"\
  -H "Authorization: bearer $ACESS_KEY"\
  -d '{
  "id":"'"$1"'",
  "realm":"'"$1"'",
  "displayName":"'"$1"'",
  "enabled": true,
  "sslRequired": "external",
  "registrationAllowed": false,
  "loginWithEmailAllowed": true,
  "duplicateEmailsAllowed": false,
  "resetPasswordAllowed": false,
  "editUsernameAllowed": false,
  "bruteForceProtected": true,
}'
echo creating client

curl -X POST "http://localhost:8080/auth/admin/realms/$1/clients/"\
  -H "Content-Type: application/json"\
  -H "Authorization: bearer $ACESS_KEY"\
  -d '{"clientId" : "'"$2"'",
      "name" :"'"$2"'",
    }'
      # "redirectUris : [
      # "http://127.0.0.1/"'"$1"'"/auth/google/final-callback",
      # "http://127.0.0.1/"'"$1"'"/auth/github/final-callback"
      # ]

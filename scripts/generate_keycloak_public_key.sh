#!/bin/bash
if [ $# -eq 0 ]
  then
    echo "please enter provider id"
    exit
fi
echo '-----BEGIN PUBLIC KEY-----' > .public_key.txt;
curl http://localhost:8080/auth/realms/"$1" | jq '.public_key' | xargs echo  >> .public_key.txt;
echo '-----END PUBLIC KEY-----' >> .public_key.txt
base64 ./.public_key.txt | xargs echo > .public_key_base64.txt
PK=$(cat ./.public_key_base64.txt)
# sed -i 's/(?KEYCLOAK_PK<=).*$/hello/g' ../bitloops-rest/docker.env
sed -i.bak "s/^KEYCLOAK_PK.*$/KEYCLOAK_PK=$PK/g" ../bitloops-rest/docker.env 

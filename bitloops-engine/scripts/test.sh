function startDocker(){
  shopt -s lastpipe
  docker ps | grep -o "$1" | read ISOPEN
  if ! [[ -z "$ISOPEN" ]]
    then 
      echo "$1 is already initialized ..."
      return
  fi
  echo "initiazing $1 ..."
  docker ps -a | grep "$1" | grep -o '^\S*' | xargs -I{} docker start {} 
}
startDocker redis
startDocker nats
echo starting tests ...

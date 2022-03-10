shopt -s lastpipe
if [ $# -eq 0 ]
  then
    echo "please insert the module you want to test"
fi
docker ps | grep -o "$1" | read ISOPEN
if ! [[ -z "$ISOPEN" ]]
then 
  echo "$1 is already initialized ..."
  exit
fi
echo "initiazing $1 ..."
docker ps -a | grep "$1" | grep -o '^\S*' | xargs -I{} docker start {} 


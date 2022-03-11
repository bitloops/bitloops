while getopts ":t:" opt; do
  case $opt in
	  t)
      tag=$OPTARG
        ;;
    \?)
      echo "Invalid option: -$OPTARG" >&2
      exit 1
      ;;
    :)
      echo "Option -$OPTARG requires an argument." >&2
      exit 1
      ;;
  esac
done

docker build -t bitloops/rest:$tag .
docker push bitloops/rest:$tag
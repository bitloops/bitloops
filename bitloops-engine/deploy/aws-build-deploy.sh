# check for flags
while getopts ":r:c:v:t:" opt; do
  case $opt in
  	t)
	  tag=$OPTARG
      ;;
  	v)
	  values=$OPTARG
      ;;
  	c)
	  clusterName=$OPTARG
      ;;
	r)
	  region=$OPTARG
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

# build and push the image
sh ./deploy/build-push-image.sh -t $tag

# deploy to AWS
sh ./deploy/aws-deploy.sh -r $region -c $clusterName -v $values
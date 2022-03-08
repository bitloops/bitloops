# check for flags
while getopts ":r:c:v:f:" opt; do
  case $opt in
	r)
	  region=$OPTARG
      ;;
	c)
	  clusterName=$OPTARG
      ;;
	v)
	  values=$OPTARG
      ;;
  f)
	  chartFileName=$OPTARG
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

# update kubeconfig
aws eks update-kubeconfig --region "$region" --name "$clusterName"

# # navigate to charts/bitloops-engine
cd charts/bitloops-engine
helm package .

# check if already exists
curl --data-binary "@"$chartFileName https://aws.chartmuseum.dev.bitloops.net/api/charts -u admin:admin

# delete file
rm -f $chartFileName

# # deploy bitloops-engine
helm upgrade --install bitloops-engine bitloops-engine/bitloops-engine --values $values
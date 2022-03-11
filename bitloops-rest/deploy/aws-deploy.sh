while getopts ":p:c:r:t:f:u:" opt; do
  case $opt in
	 t)
	  tag=$OPTARG
      ;;
    c)
	  clusterName=$OPTARG
      ;;
    r)
	  region=$OPTARG
      ;;
    p)
      valuesPath=$OPTARG
      ;;
    f)
      chartFileName=$OPTARG
      ;;
    u)
      userCreds=$OPTARG
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

# eks update kubekonfig
aws eks update-kubeconfig --region $region  --name $clusterName

# navigate to charts/bitloops-rest
cd charts/bitloops-rest
helm package .
curl --data-binary "@"$chartFileName http://aws.chartmuseum.dev.bitloops.net/api/charts -u $userCreds

# delete file
rm -f $chartFileName
helm repo add bitloops-rest http://aws.chartmuseum.dev.bitloops.net --username admin --password admin
helm repo update

# deploy helm charts
helm upgrade --install bitloops-rest bitloops-rest/bitloops-rest \
--values=$valuesPath --set image.tag=$tag
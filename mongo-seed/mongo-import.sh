echo "Importing workflows..." &&
mongoimport --host mongodb --db bitloops_managed --collection workflows --drop --file /workflows.json --jsonArray &&
echo "Importing services..." &&
mongoimport --host mongodb --db bitloops_managed --collection services --drop --file /services.json --jsonArray &&
echo "Importing workspaces..." &&
mongoimport --host mongodb --db bitloops_managed --collection workspaces --drop --file /workspaces.json --jsonArray &&
echo "Importing environments..." &&
mongoimport --host mongodb --db bitloops_managed --collection environments --drop --file /environments.json --jsonArray &&
echo "Importing secrets..." &&
mongoimport --host mongodb --db bitloops_managed --collection secrets --drop --file /secrets.json --jsonArray

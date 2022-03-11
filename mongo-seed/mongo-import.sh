
mongoimport --host mongodb --db bitloops_managed --collection workflows --drop --file /workflows.json --jsonArray &
mongoimport --host mongodb --db bitloops_managed --collection services --drop --file /services.json --jsonArray
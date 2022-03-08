FROM node:16-alpine3.12

WORKDIR /usr/src/app

RUN npm install -g typescript@4.5.2 

COPY package*.json ./

RUN npm install

COPY . .

EXPOSE 8080 8080

#Build to project
RUN npm run build

# Run node server
CMD npm run start
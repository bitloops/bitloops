FROM node:lts-alpine3.12

WORKDIR /usr/src/app

RUN npm install -g typescript

COPY package*.json ./

RUN npm install

COPY . .

EXPOSE 8080 80

#Build the project
RUN npm run build

# Run node server
CMD npm run start

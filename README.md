![Bitloops](https://bitloops.com/assets/img/bitloops-logo_320x80.png)

# Bitloops

Bitloops is a scalable open source Firebase substitute that can support **any** database and workflow orchestration. We’re building Bitloops using enterprise-grade open source tools so that you can build any type of enterprise-grade application 10x faster that will connect and serve your web, mobile, desktop, or server applications seamlessly.

### liveSync

![liveSync](https://github.com/bitloops/bitloops/blob/f2c77aad338bca10c1338d38e807a0da665fe9f8/docs/assets/Bitloops-helloWorld.gif)

_Create and update strongly-typed HTTP/2 REST APIs in seconds and use them instantly as RPCs on your client code with just a single line of code (only available for TypeScript projects currently)_

### Subscriptions to Realtime Events

![Subscriptions](https://github.com/bitloops/bitloops/blob/722de0c25c0538a5d529bf5c64faf25f9961f701/docs/assets/subscription-react.png)

_Bitloops allows your client code to subscribe to operations of any database or backend event with just a single line of code_

### Authentication

![Authentication](https://github.com/bitloops/bitloops/blob/722de0c25c0538a5d529bf5c64faf25f9961f701/docs/assets/auth-react.png)

_Add powerful authentication to your clients with a single line of code_

- Backend-as-a-Service: authentication, realtime database, storage (soon), hosting, subscriptions and notifications, cloud infrastructure
- Low-code (drag & drop) IDE that allows developers to build actual backend services using Picoservices or traditional Microservices or APIs
- Instant code deployment into development, testing or production environments
- Integration platform-as-a-service that allows integration of any application, microservice or 3rd party service

Bitloops removes the complexity and repetitiveness required to build and deploy a modern backend application, API or database from scratch and allows you to continue scaling and extending your applications faster than any other tool.

Using Bitloops, you can easily integrate your application with user authentication & multiple sign-in methods, integration of any database, data storage (soon), as well as hosting for your website. In addition, you can continue building new products and features, or iterate existing features / business logic 10x faster than with traditional tools.

Bitloops is the only platform you will need for all your backend needs. We are still at our early stages with much to do and a long backlog but with your support we will get there faster.

## Why should I care?

- [x] **liveSync**: Your frontend is connected and synced with your backend even during development allowing for seamless integration (think of GraphQL Apollo on steroids)
- [x] **Realtime Everything**: Receive events on your frontend with a single line of code and turn all your databases into “realtime” ones
- [x] **10x Productivity**: Creating APIs and backends just became 10 times faster as we take care of all the boilerplate and infrastructure
- [x] **Authentication**: Authentication with a single line of code
- [x] **Hosting**: Host and deploy your frontend application just by running "bitloops deploy”
- [ ] **Storage**: Store your files
- [x] **Scalability**: We are building Bitloops with parallel processing and scalability in mind Everything scales horizontally on Kubernetes
- [ ] **Security**: We ensure that best practices are followed in every step to ensure security in everything you build

## Low-Code Workflow Orchestration & Picoservices Architecture - Why should I care?

- [x] **Instant deployments**: Deploy your backend changes to production in milliseconds
- [x] **Reusability**: Write only unique and high value code and maximize reusability
- [ ] **Powerful Collaboration Features**: Develop in a collaborative way, Google Docs-style
- [ ] **Versioning & History**: No need for repos and commits everything is stored automatically for you
- [ ] **Out-of-the-box EDA**: Out of the box delivery guarantee strategies and error handling for Event Driven systems
- [ ] **Easy maintenance**: Automatic package and library updates minimizing maintenance effort
- [x] **Low-Code Business Logic**: Write your business logic in a language agnostic and timeless manner using Low-Code diagrams
- [x] **Polyglot Applications**: Leverage any package and library written in any language - use the best for the job and create truly polyglot applications
- [x] **Backwards Compatible**: Connect your existing services and APIs using REST, gRPC, Kafka and more

## Status

- [x] Alpha: We are testing Bitloops with a closed set of users - Low testing coverage, several bugs, missing features, UX experience suboptimal. We are aware of most important issues and we are wokring hard to fix them but we try to maintain the right balance between releasing fast and iterating with writing high quality code
- [ ] Public Alpha - Community Edition - Coming Soon!: Anyone can sign up over at [console.bitloops.com](https://console.bitloops.com). But go easy on us ❤️ We have big dreams but we are a small team and we just started the project in July '21.
- [ ] Public Alpha - Managed Backend-as-a-Service: Anyone can sign up over at [console.bitloops.com](https://console.bitloops.com).
- [ ] Public Beta: Stable enough for most non-enterprise use-cases
- [ ] Public: Production-ready

# Installation

## Prequisitions

- node (we suggest to install via nvm)

```bash
nvm install node
```

- docker [install from docker site](https://docs.docker.com/get-docker/)
- docker-compose

In order to validate that the above are installed correctly, running the commands below at the terminal should return the corresponding versions:

```bash
node -v
docker -v
docker-compose -v
```

To check if docker is up and running you may check by running the command below, without getting an error:

```bash
docker ps
```

### Step 1

```bash
# Install bitloops cli
npm install -g bitloops-cli

# or using yarn
yarn global add bitloops-cli
```

### Step 2

```bash
# For user authentication
bitloops login
```

### Step 3

```bash
# To create a workspace and give it a name 
# This may take a while the first time you run it, since it will have to pull the necessary images from Docker Hub. So please try to be a bit patient 🙏.

bitloops install -n "<Workspace Name>"
```

**After the completion of this step, your Workspace Id will appear, in order to copy it and use it in the next step.**

### Step 4

```bash
# Establish a connection between bitloops console and your local installation
bitloops tunnel -w "<Workspace Id>"
```

### Step 5

**Visit the [Bitloops Console](https://console.bitloops.com/login), and login with Google with the same account you logged in during Step 2.**
## On Windows

You need to have a terminal which can run bash commands e.g. [git bash](https://gitforwindows.org/), and use this to execute the bitloops commands. After install a restart may be required to be able to use docker commands.

## On Linux

Currently this library uses libsecret so you may need to install it before running.  
**You should follow Step 1 from above, then depending on your distribution, you will need to run the following command:**

- Debian/Ubuntu: `sudo apt-get install libsecret-1-dev`
- Red Hat-based: `sudo yum install libsecret-devel`
- Arch Linux-based:
  - `sudo pacman -S libsecret`
  - `sudo pacman -S gnome-keyring`

**Continue with Step 2, 3, 4 and 5 from above.**

# Contents

Bitloops' monorepo contains the following projects:

- [Bitloops Engine](https://github.com/bitloops/bitloops/tree/main/bitloops-engine)
- [Bitloops REST](https://github.com/bitloops/bitloops/tree/main/bitloops-rest)

# Licenses

Each project in this repo contains licensing information specific to that project.

import { IDatabase } from './interfaces/index';
import { MongoClient } from 'mongodb';
import { Options } from '.';
import {
	EventTriggerWorkflowInfo,
	IBitloopsWorkflowDefinition,
	WorkspaceSecretsInfo,
	WorkspaceServicesInfo,
} from '../entities/workflow/definitions';
import { replaceAllCharacters } from '../util/stringManipulator';

// Connection URL
const baseUrl = 'mongodb://localhost:27017';

// Database Name
const DB_NAME = 'bitloops_managed';
const MONGO_ID_DELIMITER = ':';

class Mongo {
	private client: MongoClient;

	constructor() {
		const urlPrefix = Options.getOption('MONGO_URL_PREFIX');
		const urlSuffix = Options.getOption('MONGO_URL_SUFFIX');
		const user = Options.getOption('MONGO_USER');
		const pass = Options.getOption('MONGO_PASSWORD');

		let url: string;
		if (urlPrefix && urlSuffix) {
			url = `${urlPrefix}${urlSuffix}`;
			if (user && pass) url = `${urlPrefix}${user}:${pass}${urlSuffix}`;
		} else url = baseUrl;
		const client = new MongoClient(url);
		this.client = client;
	}
	async connect() {
		// Use connect method to connect to the server
		console.info('Connecting to Mongo...');
		await this.client.connect();
		console.info('Connected successfully to MongoDB server');
	}

	async disconnect() {
		await this.client.close();
		console.info('Disconnected successfully from MongoDB server');
	}

	async getWorkflowsTriggeredByEvent(
		workspaceId: string,
		messageId: string,
	): Promise<EventTriggerWorkflowInfo[] | null> {
		const collectionName = 'workflow_event_triggers';
		const mongoId = `${workspaceId}${MONGO_ID_DELIMITER}${messageId}`;

		const collection = this.client.db(DB_NAME).collection(collectionName);
		const findResult = await collection.findOne({ _id: mongoId });
		if (findResult === null) {
			return null;
		}
		return findResult.workflows;
	}

	async getWorkflow(workflowId: string, version?: string): Promise<IBitloopsWorkflowDefinition | null> {
		const collectionName = 'workflows';
		const mongoId = workflowId; // `${workspaceId}${MONGO_ID_DELIMITER}${workflowId}`;

		const collection = this.client.db(DB_NAME).collection(collectionName);
		const findLatestVersionResult = await collection.findOne(
			{ _id: mongoId },
			{ projection: { _id: 0, latestVersion: 1 } },
		);
		if (!findLatestVersionResult) return null;
		const { latestVersion } = findLatestVersionResult;
		const findVersion: string = version ? version : latestVersion;

		const projectionField = `versions.${findVersion}`;
		const { versions, workspaceId } = await collection.findOne(
			{ _id: mongoId },
			{ projection: { _id: 0, [projectionField]: 1, workspaceId: 1 } },
		);
		const workflowDefinition: IBitloopsWorkflowDefinition = versions[findVersion];
		if (!workflowDefinition) return null;
		workflowDefinition.version = findVersion;
		workflowDefinition.workspaceId = workspaceId;
		workflowDefinition.id = workflowId;
		workflowDefinition.bitloopsEngineVersion = replaceAllCharacters(
			workflowDefinition.bitloopsEngineVersion,
			'.',
			'',
		);
		return workflowDefinition ?? null;
	}

	async getWorkflowServices(
		workspaceId: string,
		servicesIds: string[],
		environmentId: string,
	): Promise<Record<string, WorkspaceServicesInfo>> {
		const collectionName = 'services';
		const collection = this.client.db(DB_NAME).collection(collectionName);
		// const objectIds = servicesIds.map((serviceId) => new ObjectId(serviceId));
		// const projectionField = `environments.${environmentId}`;
		const findResults = await collection.find({ _id: { $in: servicesIds }, workspaceId: workspaceId }).toArray();
		const resObject = findResults.reduce((prevValue, currValue) => {
			currValue.id = currValue._id;
			delete currValue._id;
			return { ...prevValue, [currValue.id]: currValue };
		}, {});
		// console.log('Mongo Array', findResults, 'to object=>', resObject);
		return resObject;
	}

	async getSecretsById(workspaceId: string, secretIds: string[]): Promise<Record<string, WorkspaceSecretsInfo>> {
		const collectionName = 'secrets';
		const collection = this.client.db(DB_NAME).collection(collectionName);

		const findResults = await collection.find({ _id: { $in: secretIds }, workspaceId: workspaceId }).toArray();
		const resObject = findResults.reduce((prevValue, currValue) => {
			currValue.id = currValue._id;
			delete currValue._id;
			return { ...prevValue, [currValue.id]: currValue };
		}, {});
		return resObject;
	}
}

export default Mongo;

// (async () => {
// 	const db = new Mongo();
// 	await db.connect();
// 	const services = ['serviceId1', 'serviceId2'];
// 	// const services = ['serviceId3'];
// 	const workspaceId = 'db24bb48-d2e3-4433-8fd0-79eef2bf63df';
// 	// const workspaceId = 'ae74bb48-d2e3-6872-8fd1-79eef9bf63dc';
// 	const environmentId = 'prod_345';
// 	// const environmentId = 'dev_142'
// 	const res = await db.getWorkflowServices(workspaceId, services, environmentId);
// 	await db.disconnect();
// })();

// const sampleServices = [
// 	{
// 		workspaceId: 'db24bb48-d2e3-4433-8fd0-79eef2bf63df',
// 		_id: 'serviceId1',
// 		name: 'redis',
// 		description: 'gRPC Mongo Service',
// 		tags: ['db', 'mongo', 'gRPC'],
// 		type: 'grpc',
// 		proto: '',
// 		environments: {
// 			prod_345: {
// 				target: 'localhost:3444',
// 				ssl: false,
// 			},
// 			dev_142: {
// 				target: 'bitloops.net:3444',
// 				ssl: true,
// 			},
// 		},
// 	},
// 	{
// 		workspaceId: 'db24bb48-d2e3-4433-8fd0-79eef2bf63df',
// 		_id: 'serviceId2',
// 		name: 'Mongo',
// 		description: 'gRPC Mongo Service',
// 		tags: ['db', 'mongo', 'gRPC'],
// 		type: 'grpc',
// 		proto: '',
// 		environments: {
// 			prod_345: {
// 				target: 'localhost:3444',
// 				ssl: false,
// 			},
// 			dev_142: {
// 				target: 'bitloops.net:3444',
// 				ssl: true,
// 			},
// 		},
// 	},
// 	{
// 		workspaceId: 'ae74bb48-d2e3-6872-8fd1-79eef9bf63dc',
// 		_id: 'serviceId3',
// 		name: 'Mongo',
// 		description: 'gRPC Mongo Service',
// 		tags: ['db', 'mongo', 'gRPC'],
// 		type: 'grpc',
// 		proto: '',
// 		environments: {
// 			prod_345: {
// 				target: 'localhost:3444',
// 				ssl: false,
// 			},
// 			dev_142: {
// 				target: 'bitloops.net:3444',
// 				ssl: true,
// 			},
// 		},
// 	},
// ];

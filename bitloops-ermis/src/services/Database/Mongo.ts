import { MongoClient } from 'mongodb';
import { Options } from '..';
import { IXApiKeyDefinition } from './definitions';
import { IDatabase } from './interfaces';

// Connection URL
const baseUrl = 'mongodb://localhost:27017';

// Database Name
const DB_NAME = 'bitloops_managed';

class Mongo implements IDatabase {
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
		// console.log(url);
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

	async getXApiKey(xApiKey: string): Promise<IXApiKeyDefinition | null> {
		const collectionName = 'x_api_keys';
		const mongoId = xApiKey;

		const collection = this.client.db(DB_NAME).collection(collectionName);
		const findResult = (await collection.findOne({ _id: mongoId })) as IXApiKeyDefinition;
		return findResult ?? null;
	}
}

export default Mongo;

import Redis from '../Redis';
let redis = new Redis();
beforeEach(async () => {
	redis = new Redis();
	await redis.initializeConnection();
});
describe('connection settings', () => {
	jest.setTimeout(10000);
	it('should resolve redis connection', async () => {
		let connection = expect(redis.getConnection());
		let resolved_connection = connection.resolves;
		return resolved_connection.not.toThrowError();
	});
	it('should sucessfully close redis connection', async () => {
		let closed_connection = expect(redis.closeConnection());
		let resolved_connection = closed_connection.resolves;
		return resolved_connection.not.toThrowError();
	});
});
describe('addition and return of id info', () => {
	let topicIdInfo = {
		workspaceId: 'hey',
		topic: 'cars',
	};
	let connection_id = '34';
	it('should add connection id info', async () => {
		let insertion = expect(redis.addConnectionIdToTopic(topicIdInfo, connection_id));
		return insertion.resolves.not.toThrowError();
	});
	it('should return connection id info', async () => {
		let response = expect(redis.getConnectionIdsSubscribedToTopic(topicIdInfo.workspaceId, topicIdInfo.topic));
		return response.resolves.not.toBeUndefined();
	});
	it('should return unsubscribe from topic ', async () => {
		let unsubscription = expect(
			redis.handleTopicUnsubscribe(connection_id, topicIdInfo.workspaceId, topicIdInfo.topic),
		);
		return unsubscription.resolves.not.toThrowError();
	});
});

describe('addition and removal of topic value info', () => {
	let topicValueInfo = {
		workspaceId: 'hey',
		topics: ['cars', 'motobikes', 'bikes', 'strollers'],
	};
	let connection_id = '34';
	it('should add connection of topic value info', async () => {
		let insertion = expect(redis.addTopicsToConnectionId(connection_id, topicValueInfo));
		return insertion.resolves.not.toThrowError();
	});
	it('should return connection of topic value info', async () => {
		let response = expect(redis.getConnectionIdValue(connection_id));
		return response.resolves.not.toBeUndefined();
	});
	it('should return unsubscribe from topic ', async () => {
		let topic_to_unsubscribe = 'strollers';
		let unsubscription = expect(
			redis.handleTopicUnsubscribe(connection_id, topicValueInfo.workspaceId, topic_to_unsubscribe),
		);
		return unsubscription.resolves.not.toThrowError();
	});
});

describe('storage and removal of record', () => {
	let record = {
		hello: 'panos',
		goodbye: 'panos',
	};
	let connection_id = '34';
	it('should store connection of record', async () => {
		let insertion = expect(redis.storeConnectionIdValue(connection_id, record));
		return insertion.resolves.not.toThrowError();
	});
	it('should remove connection of record', async () => {
		let removal = expect(redis.removeConnectionId(connection_id));
		// await redis.closeConnection();
		return removal.resolves.not.toThrowError();
	});
});

describe('addition and cleaning of podid connection', () => {
	let podid = '4';
	let connection_id = '34';
	it('should add podid connection ', async () => {
		let insertion = expect(redis.addConnectionToPodId(podid, connection_id));
		return insertion.resolves.not.toThrowError();
	});
	it('should clean podid connection', async () => {
		let cleaning = expect(redis.cleanPodState(podid));
		return cleaning.resolves.not.toThrowError();
	});
});

// it('should reject connection with counterfit credentials', async () => {
//         let envRedisOptions = {
//                 host : "128.0.0.2",
//                 port : 3000,
//                 username : "counterfit",
//                 password : "counterfit",
//         };
//         let redis = new Redis();
//         await redis.initializeConnection();
//         let connection = redis.getConnection()
//         redis.getConnectionIdValue("f")
//         await redis.closeConnection()
// expect(expect(connection).rejects.toThrowError())
// let rejected_connection = connection.rejects
// return rejected_connection.toThrowError();
// });

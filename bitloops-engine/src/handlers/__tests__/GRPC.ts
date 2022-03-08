// TODO fix it
// import { GrpcTaskNode, JSONGrpcDecodedObject, NodeType } from './../../definitions';
// import GRPC from '../GRPC';
// import { Options } from '../../services';
// import { JSONCodec, NatsConnection } from 'nats';
// import { unlinkSync } from 'fs';
// import NATS from '../../services/NATS';
// import * as grpc from '@grpc/grpc-js';
// import * as protoLoader from '@grpc/proto-loader';
// import { tmpdir } from 'os';
// import { writeStringDataToFile } from '../../helpers/file';
// import { v4 as uuid } from 'uuid';

// const GRPC_SERVER_PORT = 8083;
// const PUBLISHING_TOPIC = Options.getOption('ENGINE_NATS_TOPIC');
// const PROTO_STRING = `
// syntax = "proto3";
// package randomPackage;
// service Random {rpc PingPong (PingRequest) returns (PongResponse) {};}
// message PingRequest {string message = 1;}
// message PongResponse {string message = 1;}`;

// const node: GrpcTaskNode = {
// 	id: '',
// 	name: 'grpc-test',
// 	type: NodeType.TaskNode,
// 	visual: { x: 0, y: 0, colour: null },
// 	parameters: {
// 		action: 'PingPong',
// 		interface: {
// 			grpcPackage: 'randomPackage',
// 			grpcService: 'Random',
// 			proto: PROTO_STRING,
// 			target: `0.0.0.0:${GRPC_SERVER_PORT}`,
// 		},
// 		input: { message: '`Ping`' },
// 	},
// 	service: '',
// 	serviceVersion: '',
// 	executed: false,
// 	variables: [{ name: 'firstVar', evalValue: '1' }],
// 	output: [{ name: '', evalValue: '' }],
// };

// const message = {
// 	node,
// 	variables: '',
// 	secrets: '',
// 	workflow: '',
// };

// const sleep = (ms: number) => {
// 	return new Promise((resolve) => setTimeout(resolve, ms));
// };
// function deepCloneObj(a: Record<string, unknown>) {
// 	return JSON.parse(JSON.stringify(a));
// }

// async function subscribeAndRead(nc: NatsConnection, topic: string, matchValue: string) {
// 	const js = JSONCodec();
// 	const mainChannel = nc.subscribe(topic);

// 	for await (const m of mainChannel) {
// 		const decodedMsg: Partial<JSONGrpcDecodedObject> = js.decode(m.data);
// 		// // console.log(`[${mainChannel.getProcessed()}]: ${decodedMsg}`);
// 		expect(decodedMsg.node.name).toEqual(matchValue);
// 	}
// 	// console.log('subscription closed');
// }

// async function initializeGrpcServer(port: number, protoFileContent: string): Promise<grpc.Server> {
// 	const tempFileName = uuid() + '.proto';
// 	const tempFilePath = `/${tmpdir()}/${tempFileName}`;
// 	const fileResult = await writeStringDataToFile(tempFilePath, protoFileContent);
// 	if (fileResult[0] === false) {
// 		console.error(fileResult[1]);
// 		return Promise.reject(fileResult[1]);
// 	}
// 	// console.log(`Created temp .proto file: ${tempFilePath}`);

// 	const packageDef = protoLoader.loadSync(tempFilePath);
// 	const grpcObj = grpc.loadPackageDefinition(packageDef);
// 	const randomPackage = grpcObj.randomPackage;
// 	const server = new grpc.Server();
// 	const evalString = `server.addService(randomPackage.Random.service, {
// 			PingPong: (req, res) => {
// 				// console.log('Server received:', req.request);
// 				res(null, {message: "Pong" })
// 			},
// 		})`;
// 	eval(evalString);
// 	unlinkSync(tempFilePath);

// 	return new Promise((resolve, reject) => {
// 		server.bindAsync(`0.0.0.0:${port}`, grpc.ServerCredentials.createInsecure(), (err, port) => {
// 			if (err) {
// 				console.error(err);
// 				return reject(err);
// 			}
// 			// console.log(`Server started on port ${port}`);
// 			server.start();
// 			return resolve(server);
// 		});
// 	});
// }

// describe('GRPC', () => {
// 	const js = JSONCodec();
// 	let server: grpc.Server;

// 	beforeAll(async () => {
// 		process.env['ENV'] = 'dev';
// 		server = await initializeGrpcServer(GRPC_SERVER_PORT, PROTO_STRING);
// 	});

// 	describe('connecting without error', () => {
// 		it('should publish message on NATS, and then grpc client should send input to gRPC Server', async () => {
// 			expect.assertions(3);

// 			// grpc objects use same nats instance
// 			const nats = new NATS();
// 			const nc = await nats.getConnection();
// 			const grpc = new GRPC(nats);

// 			nc.subscribe(Options.getOption('ENGINE_GRPC_NATS_TOPIC'), {
// 				callback: async (err, msg) => {
// 					const boundCallback = grpc.callback.bind(grpc);
// 					await expect(boundCallback(err, msg)).resolves.not.toThrowError();
// 				},
// 			});

// 			// dont await
// 			subscribeAndRead(nc, PUBLISHING_TOPIC, message.node.name);

// 			await expect(
// 				nats.publish(Options.getOption('ENGINE_GRPC_NATS_TOPIC'), js.encode(message)),
// 			).resolves.not.toThrowError();

// 			await sleep(2000);
// 			await nc.drain();
// 		});
// 	});

// 	describe('failing conditions', () => {
// 		it('should not read a published output on wrong server address', async () => {
// 			expect.assertions(2);
// 			const nats = new NATS();
// 			const nc = await nats.getConnection();
// 			const grpc = new GRPC(nats);

// 			const wrongMessage = deepCloneObj(message);
// 			wrongMessage.node.name = 'fail-grpc-test1';
// 			wrongMessage.node.type.parameters.interface.target = '0.0.0.127:8080';

// 			nc.subscribe(Options.getOption('ENGINE_GRPC_NATS_TOPIC'), {
// 				callback: async (err, msg) => {
// 					const boundCallback = grpc.callback.bind(grpc);
// 					await expect(boundCallback(err, msg)).resolves.not.toThrowError();
// 				},
// 			});

// 			subscribeAndRead(nc, PUBLISHING_TOPIC, wrongMessage.node.name);

// 			await expect(
// 				nats.publish(Options.getOption('ENGINE_GRPC_NATS_TOPIC'), js.encode(wrongMessage)),
// 			).resolves.not.toThrowError();

// 			await sleep(2000);
// 			await nc.drain();
// 		});

// 		it('should reject with error on protoLoader.loadSync file with illegal tokens', async () => {
// 			expect.assertions(2);
// 			const nats = new NATS();
// 			const nc = await nats.getConnection();
// 			const grpc = new GRPC(nats);

// 			const wrongMessage = deepCloneObj(message);
// 			wrongMessage.node.name = 'fail-grpc-test2';
// 			wrongMessage.node.type.parameters.interface.proto = PROTO_STRING + '";";;';

// 			nc.subscribe(Options.getOption('ENGINE_GRPC_NATS_TOPIC'), {
// 				callback: async (err, msg) => {
// 					const boundCallback = grpc.callback.bind(grpc);
// 					try {
// 						await boundCallback(err, msg);
// 					} catch (error) {
// 						expect(error.message).toMatch(new RegExp('^illegal token'));
// 					}
// 				},
// 			});

// 			subscribeAndRead(nc, PUBLISHING_TOPIC, wrongMessage.node.name);

// 			await expect(
// 				nats.publish(Options.getOption('ENGINE_GRPC_NATS_TOPIC'), js.encode(wrongMessage)),
// 			).resolves.not.toThrowError();

// 			await sleep(2000);
// 			await nc.drain();
// 		});

// 		it('should reject with error when not declaring a package', async () => {
// 			expect.assertions(3);
// 			const nats = new NATS();
// 			const nc = await nats.getConnection();
// 			const grpc = new GRPC(nats);

// 			const wrongMessage = deepCloneObj(message);
// 			wrongMessage.node.name = 'fail-grpc-test3';
// 			wrongMessage.node.type.parameters.interface.grpcPackage = null;

// 			nc.subscribe(Options.getOption('ENGINE_GRPC_NATS_TOPIC'), {
// 				callback: async (err, msg) => {
// 					const boundCallback = grpc.callback.bind(grpc);
// 					try {
// 						await boundCallback(err, msg);
// 					} catch (error) {
// 						expect(error.name).toMatch('TypeError');
// 						expect(error.message).toMatch('loadedPackageDefinition.Random is not a constructor');
// 					}
// 				},
// 			});

// 			subscribeAndRead(nc, PUBLISHING_TOPIC, wrongMessage.node.name);

// 			await expect(
// 				nats.publish(Options.getOption('ENGINE_GRPC_NATS_TOPIC'), js.encode(wrongMessage)),
// 			).resolves.not.toThrowError();

// 			await sleep(2000);
// 			await nc.drain();
// 		});
// 	});

// 	afterAll(() => {
// 		server.forceShutdown();
// 	});
// });

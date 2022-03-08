import * as fs from 'fs';
import * as os from 'os';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import { v4 as uuid } from 'uuid';
import { replaceVars } from '../../helpers/replaceVariables';
import { writeStringDataToFile } from '../../helpers/file';
import { IMQ, ILRUCache } from '../../services/interfaces';
import { Options } from '../../services';
import { MQTopics } from '../../constants';
import GrpcCache from '../../services/Cache/GrpcCache';
import { IGRPCResponse, JSONGrpcDecodedObject } from './definitions';
import { IGrpcTaskNode } from '../../entities/nodes/definitions';
import { IBitloopsWorkflowDefinition, WorkflowParams } from '../../entities/workflow/definitions';

class GRPC {
	private nats: IMQ;
	private clients: ILRUCache<grpc.Client>;

	constructor(nats: IMQ) {
		this.nats = nats;
		this.clients = new GrpcCache(10);
	}

	async callback(data: JSONGrpcDecodedObject): Promise<void> {
		const { nodeDefinition, workflowDefinition, workflowParams } = data;
		const { constants, variables, systemVariables } = workflowParams;
		// console.log(`Making ${data.nodeDefinition.name} gRPC call for ${variables.instanceId}`);
		let gRPCResponse = await this.makeCall(nodeDefinition, workflowDefinition, workflowParams);
		// TODO (later) feature add error handling
		if (gRPCResponse.error) {
			// console.log(`Received error in ${node.name} gRPC response for ${variables.instanceId}`);
			console.error(variables.instanceId, gRPCResponse.error);
			const varObject = {
				...variables,
				error: gRPCResponse.error,
			};
			// await saveState(workflow, node, varObject, 'error', redis);
		} else {
			// console.log(`Received ${nodeDefinition.name} valid gRPC response for ${variables.instanceId}`);
			// console.log('gRPC response', gRPCResponse.value);
			const replaceVarsParams = {
				variables,
				output: gRPCResponse.value,
				workflowDefinition,
				constants,
				systemVariables,
			};
			const output = await replaceVars(nodeDefinition.type.output, replaceVarsParams);
			// console.log('grpc response', gRPCResponse.value);
			// const executed = this.workflow.getParams()?.systemVariables?.nodes[this.id].executed;

			if (!systemVariables.nodes[nodeDefinition.id]) systemVariables.nodes[nodeDefinition.id] = {};
			systemVariables.nodes[nodeDefinition.id].executed = true;

			const version = Options.getVersion();
			this.nats.publish(`${version}.${Options.getOption(MQTopics.ENGINE_TOPIC)}`, {
				nodeDefinition,
				workflowParams: { constants, variables: { ...variables, ...output }, systemVariables },
				workflowDefinition,
			});
		}
		gRPCResponse = null;
	}

	private async makeCall(
		node: IGrpcTaskNode,
		workflow: IBitloopsWorkflowDefinition,
		workflowParams: WorkflowParams,
	): Promise<IGRPCResponse> {
		// TODO add connection caching and only run below lines if not cached
		// TODO add caching with x min expiration and later be able to expire through admin-topic
		// TODO add limit on the caching object so that it doesn't grow huge
		const { constants, variables, systemVariables } = workflowParams;
		const { target, grpcService, grpcPackage, ssl, proto, rpc } = node.type.parameters.interface;
		// TODO hash protofile
		const clientId = `${target}:${proto}`;
		let client = this.clients.get(clientId);

		if (!client) {
			const startedLoadingProto = Date.now();
			const tempFilename = uuid() + '.proto';
			const tempFilePath = `/${os.tmpdir()}/${tempFilename}`;
			const fileResult = await writeStringDataToFile(tempFilePath, eval(proto));
			const wroteFileDuration = Date.now() - startedLoadingProto;
			if (fileResult[0] !== true) return { value: null, error: fileResult[1] };
			const packageDefinition = protoLoader.loadSync(`/${os.tmpdir()}/${tempFilename}`, {
				keepCase: true,
				longs: String,
				enums: String,
				defaults: true,
				oneofs: true,
			});
			const loadedFileDuration = Date.now() - startedLoadingProto - wroteFileDuration;
			let loadedPackageDefinition = grpc.loadPackageDefinition(packageDefinition);
			const evalString = `new loadedPackageDefinition${grpcPackage ? '.' + grpcPackage : ''
				}.${grpcService}('${target}', ${ssl ? 'grpc.credentials.createSsl()' : 'grpc.credentials.createInsecure()'
				})`;
			client = eval(evalString);
			this.clients.set(clientId, client);
			const createdClientDuration = Date.now() - startedLoadingProto - wroteFileDuration - loadedFileDuration;
			fs.unlinkSync(tempFilePath);
			loadedPackageDefinition = null;
		}
		const replaceVarsParams = { variables, workflow, constants, systemVariables };
		const input = await replaceVars(node.type.parameters.interface.input, replaceVarsParams);
		// console.log('gRPC INPUT', input);
		// TODO feature add proper error handling
		const gRPCResponse = await new Promise((resolve, reject) => {
			return client[rpc](input, (error: Error, response: any): void => {
				if (error) {
					console.error(error);
					return reject(error);
				} else return resolve(response);
			});
		})
			.then((value) => {
				return { value, error: null };
			})
			.catch((error) => {
				return { value: null, error };
			});

		client = null;
		// TODO handle kpis on cache hit/miss
		// const gotResponseDuration =
		// 	Date.now() - startedLoadingProto - wroteFileDuration - loadedFileDuration - createdClientDuration;

		// console.log(
		// 	`${node.name} gRPC times`,
		// 	'wroteFile:',
		// 	wroteFileDuration,
		// 	'loadedFile:',
		// 	loadedFileDuration,
		// 	'createdClient: ',
		// 	createdClientDuration,
		// 	'gotResponse',
		// 	gotResponseDuration,
		// );
		return gRPCResponse;
	}
}

export default GRPC;

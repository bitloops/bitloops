import { ITypedStringVariable, variableTypes, WorkflowParams } from './../../entities/workflow/definitions';
// import { v4 as uuid } from 'uuid';
import { replaceVars } from '../../helpers/replaceVariables';
import { IMQ } from '../../services/interfaces';
import { Options } from '../../services';
import { MQTopics } from '../../constants';
import { IRestTaskNode } from '../../entities/nodes/definitions';
// import { IBitloopsWorkflowDefinition } from '../../entities/workflow/definitions';
import { URLSearchParams } from 'url';
import fetch, { BodyInit, RequestInit } from 'node-fetch';
import { replaceStringBetween } from '../../util/stringManipulator';

class REST {
	private nats: IMQ;

	constructor(nats: IMQ) {
		this.nats = nats;
	}

	async callback(data: any): Promise<void> {
		const { nodeDefinition, workflowDefinition, workflowParams } = data;
		const { constants, variables, systemVariables, context } = workflowParams;
		console.log(`Making ${nodeDefinition.name} REST call for ${variables.instanceId}`);
		let restResponse = await this.makeCall(nodeDefinition, workflowParams);
		// // TODO (later) feature add error handling
		if (restResponse.error) {
			// console.log(`Received error in ${node.name} rest response for ${variables.instanceId}`);
			console.error(variables.instanceId, restResponse.error);
			const varObject = {
				...variables,
				error: restResponse.error,
			};
			// await saveState(workflow, node, varObject, 'error', redis);
		} else {
			// console.log(`Received ${node.name} valid rest response for ${variables.instanceId}`);
			const replaceVarsParams = {
				variables,
				output: restResponse.value,
				workflowDefinition,
				constants,
				systemVariables,
				context,
			};
			const output = await replaceVars(nodeDefinition.type.output, replaceVarsParams);

			if (!systemVariables.nodes[nodeDefinition.id]) systemVariables.nodes[nodeDefinition.id] = {};
			systemVariables.nodes[nodeDefinition.id].executed = true;

			const version = Options.getVersion();
			this.nats.publish(`${version}.${Options.getOption(MQTopics.ENGINE_TOPIC)}`, {
				nodeDefinition,
				workflowParams: { constants, variables: { ...variables, ...output }, systemVariables, context },
				workflowDefinition,
			});
		}
		restResponse = null;
	}

	async makeCall(nodeDefinition: IRestTaskNode, workflowParams: WorkflowParams) {
		console.log('MAKING CALL');
		const { method, headers, body, params } = nodeDefinition.type.parameters.interface;
		let { uri, urlPath, query } = nodeDefinition.type.parameters.interface;
		if (!urlPath) urlPath = '/';
		if (urlPath.charAt(0) !== '/') urlPath = `/${urlPath}`;
		try {
			if (!uri) throw new Error('Target uri must be defined!');
			const parsedHeaders = await this.parseHeaders(headers, workflowParams);
			// console.log('body', body);
			const parsedBody: Record<string, string> = await replaceVars(body, workflowParams);
			const parsedQuery: Record<string, string> = await replaceVars(query, workflowParams);
			uri = await this.replaceUriVars(uri, workflowParams);
			uri = uri + urlPath;
			uri = await this.buildWithParams(uri, params, workflowParams);
			uri = this.buildWithQuery(uri, parsedQuery);
			console.log('final uri', uri);
			// console.log('method', method);
			// console.log('parsedBody', parsedBody);
			// console.log('parsedHeaders', parsedHeaders);
			const fetchOptions: RequestInit = {
				method,
				headers: parsedHeaders,
			};

			const builtBody = this.buildBody(parsedBody, parsedHeaders);
			if (builtBody !== null) fetchOptions.body = builtBody;
			const response = await fetch(uri, fetchOptions);
			if (!response.ok) {
				console.error(await this.parseResponseType(response));
				throw new Error(`HTTP Error Response: ${response.status} ${response.statusText}`);
			}
			const data = await this.parseResponseType(response);
			console.log('RESPONSE OK', data);
			return { value: data, error: null };
		} catch (error) {
			console.error('RESPONSE NOT OK', error);
			return { value: null, error };
		}
	}

	private async replaceUriVars(uri: string, workflowParams: WorkflowParams): Promise<string> {
		const uriTyped: ITypedStringVariable[] = [
			{
				type: variableTypes.string,
				name: 'value',
				evalValue: uri,
			},
		];
		const replacedUri = await replaceVars(uriTyped, workflowParams);
		return replacedUri.value;
	}

	private async parseHeaders(
		headers: ITypedStringVariable[],
		workflowParams: WorkflowParams,
	): Promise<Record<string, string>> {
		const parsedHeaders = await replaceVars(headers, workflowParams);
		return Object.fromEntries(Object.entries(parsedHeaders).map(([k, v]) => [k.toLowerCase(), v]));
	}

	private replaceFunction = (params: Record<string, any>) => {
		return (_match: any, p1: string) => {
			console.log('p1', p1);
			return params[p1];
		};
	};

	private async buildWithParams(
		uri: string,
		params: ITypedStringVariable[],
		workflowParams: WorkflowParams,
	): Promise<string> {
		const paramsObj = await replaceVars(params, workflowParams);
		const matchingInfo = { str: uri, before: '{{', after: '}}' };
		const replacedUri = replaceStringBetween(matchingInfo, this.replaceFunction(paramsObj));
		return replacedUri;
	}

	private buildWithQuery(uri: string, query: Record<string, string>): string {
		const keys = Object.keys(query);

		if (query && keys.length !== 0) {
			const parsedQuery = new URLSearchParams();
			for (const key of keys) {
				parsedQuery.append(key, query[key]);
			}
			return `${uri}?${parsedQuery.toString()}`;
		}
		return uri;
	}

	private async parseResponseType(response: any) {
		const responseType = response.headers.get('content-type');
		// console.log(responseType);
		if (!responseType) return response;
		const [type, subtypeAndParams] = responseType.split('/');
		// console.log(type, 'subtype:', subtypeAndParams);

		let data: any;
		switch (type) {
			case 'application':
				// TODO subtypes
				data = await response.json();
				break;
			case 'text':
				data = await response.text();
				break;
			// TODO
			case 'audio':
			case 'image':
			case 'multipart':
			case 'video':
			// X-Headers
			default:
				throw new Error('Unknown http response type');
		}
		return data;
	}

	private buildBody(body: Record<string, string>, headers: Record<string, string>): BodyInit | null {
		// body: body && body.length > 0 ? JSON.stringify(parsedBody) : null,
		if (Object.keys(body).length === 0) return null;
		console.log('headers', headers);
		switch (headers['content-type']) {
			case 'application/x-www-form-urlencoded':
				// TODO subtypes
				// data = await response.json();
				// const paramsObjecy
				return new URLSearchParams(Object.entries(body)).toString();
			case 'application/json':
				return JSON.stringify(body);
			case 'text':
				// data = await response.text();
				break;
			// TODO
			case 'audio':
			case 'image':
			case 'multipart':
			case 'video':
			// X-Headers
			default:
				throw new Error(`Unknown http content type ${headers['content-type']}`);
		}
	}
}

export default REST;

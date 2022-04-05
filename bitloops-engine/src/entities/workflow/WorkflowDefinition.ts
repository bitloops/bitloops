import {
	IGrpcTaskInterface,
	IMessageTaskInterface,
	IRestTaskInterface,
	ITaskNode,
	ITaskNodeType,
	ServicesEnum,
} from './../nodes/definitions/index';
import { IServices } from '../../services/definitions';
import { NodeTypeName } from '../nodes/definitions';
import {
	IBitloopsWorkflowDefinition,
	WorkspaceGrpcServiceInfo,
	WorkspaceRestServiceInfo,
	WorkspaceServicesInfo,
} from './definitions';
import { WorkflowMainInfo } from '../../handlers/bitloopsEngine/definitions';
import Services from '../../services';

export abstract class WorkflowDefinition {
	private static services;

	private static async getServices(): Promise<IServices> {
		WorkflowDefinition.services = Services.getServices();
		if (!WorkflowDefinition.services) {
			WorkflowDefinition.services = await Services.initializeServices();
		}
		return WorkflowDefinition.services;
	}

	public static async get(workflowMainInfo: WorkflowMainInfo): Promise<IBitloopsWorkflowDefinition> {
		let { workflowVersion } = workflowMainInfo;
		const { workflowId, environmentId } = workflowMainInfo;
		const { db, workflowCache, workflowVersionMappingCache } = await WorkflowDefinition.getServices();
		if (!workflowVersion) {
			workflowVersion = await workflowVersionMappingCache.fetch(workflowId);
		}
		let workflowDefinition: IBitloopsWorkflowDefinition = await workflowCache.fetch(
			workflowId,
			workflowVersion,
			environmentId,
		);
		console.log('Got workflow from cache')
		if (!workflowDefinition) {
			console.log('Got workflow from db')
			workflowDefinition = await db.getWorkflow(workflowId, workflowVersion);
			if (workflowDefinition === null) {
				// TODO Error handling in this function
				throw new Error(`Could not find workflow for ${workflowId} and version:${workflowVersion}`);
			}

			const services = await WorkflowDefinition.getWorkflowServices(
				workflowDefinition,
				workflowDefinition.workspaceId,
				environmentId,
			);
			WorkflowDefinition.stitchWorkflowWithServices(workflowDefinition, services, environmentId);

			workflowVersionMappingCache.cache(workflowId, workflowDefinition.version);
			workflowCache.cache(workflowId, workflowDefinition.version, environmentId, workflowDefinition);
		}
		// workflowCache.getSnapshot();
		return workflowDefinition;
	}

	private static async getWorkflowServices(
		workflowDefinition: IBitloopsWorkflowDefinition,
		workspaceId: string,
		environmentId: string,
	): Promise<Record<string, WorkspaceServicesInfo>> {
		const { db, workspaceServicesCache } = await WorkflowDefinition.getServices();

		const servicesIds = new Set<string>();
		for (const node of workflowDefinition.nodes) {
			if (node.type.name === NodeTypeName.TaskNode) {
				servicesIds.add(node.type.parameters.service);
			}
		}
		// console.log('All servicesIds needed:', servicesIds);
		const cachedServices: Record<string, WorkspaceServicesInfo> = await workspaceServicesCache.fetchServices(
			workspaceId,
			servicesIds,
		);
		const cachedServicesIds = Object.keys(cachedServices);
		const missingServicesIds = [...servicesIds].filter((id) => !cachedServicesIds.includes(id));
		const servicesToBeFetched = missingServicesIds.length > 0 ? missingServicesIds : [...servicesIds];

		let services = cachedServices;
		if (!cachedServices || missingServicesIds.length > 0) {
			// console.log('Some(or all) services missing from cache, fetching from DB, cachedServices =', cachedServices);
			const notCachedServices: Record<string, WorkspaceServicesInfo> = await db.getWorkflowServices(
				workspaceId,
				servicesToBeFetched,
				environmentId,
			);
			// console.log('Not cached services', notCachedServices);
			await workspaceServicesCache.cacheServices(workspaceId, notCachedServices);
			services = { ...services, ...notCachedServices };
		}
		return services;
	}

	private static requiredServiceInfoExist(serviceInfo: WorkspaceServicesInfo, environmentId: string) {
		if (!serviceInfo || (!serviceInfo?.environments?.[environmentId] && serviceInfo.type !== ServicesEnum.MESSAGE && serviceInfo.type !== ServicesEnum.DYNAMIC_REST)) return false;
		return true;
	}

	/** Mutates workflowDefinition */
	private static stitchWorkflowWithServices(
		workflowDefinition: IBitloopsWorkflowDefinition,
		services: Record<string, WorkspaceServicesInfo>,
		environmentId: string,
	) {
		// TODO skip if node is executed?
		for (const node of workflowDefinition.nodes) {
			if (node.type.name === NodeTypeName.TaskNode) {
				const { interface: taskInterface, service } = node.type.parameters;

				// console.log('Services service', services[service]);
				console.log('service', service);
				// console.log('environmentId', environmentId);
				// console.log('environment id', services[service][environmentId]);
				const serviceInfo = services[service];
				if (!WorkflowDefinition.requiredServiceInfoExist(serviceInfo, environmentId)) {
					console.log(serviceInfo);
					throw new Error(
						`No such service or service:${service} doesn't have environmentId:${environmentId} defined`,
					);
				}
				taskInterface.type = services[service].type;
				switch (serviceInfo.type) {
					case ServicesEnum.GRPC: {
						const interfaceRef = taskInterface as IGrpcTaskInterface;
						interfaceRef.proto = `'${serviceInfo.meta.proto}'`; // It is currently evaled(due to constants)
						interfaceRef.ssl = serviceInfo.environments[environmentId].ssl;
						interfaceRef.target = serviceInfo.environments[environmentId].target;
						break;
					}
					case ServicesEnum.REST: {
						const interfaceRef = taskInterface as IRestTaskInterface;
						// let { swagger, uri } = taskInterface as IRestTaskInterface;
						const targetBaseURL = serviceInfo.environments[environmentId].target;
						const targetSSL = serviceInfo.environments[environmentId].ssl;
						interfaceRef.uri = targetSSL ? `https://${targetBaseURL}` : `http://${targetBaseURL}`;
						interfaceRef.swagger = serviceInfo.meta.swagger;
						break;
					}
					case ServicesEnum.MESSAGE: {
						const interfaceRef = taskInterface as IMessageTaskInterface;
						interfaceRef.proto = serviceInfo.meta.proto;
						break;
					}
					case ServicesEnum.DYNAMIC_REST: {
						break;
					}
					default:
						console.error(`Task interface type in ${serviceInfo} - not implemented`);
						break;
				}
				// console.log('Stitched node type interface', node.type.parameters.interface);
			}
		}
	}
}

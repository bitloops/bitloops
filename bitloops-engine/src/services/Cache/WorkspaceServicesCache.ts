import { WorkspaceServicesInfo } from '../../entities/workflow/definitions';
import Cache from './Cache';
import { IWorkspaceServicesCache } from '../interfaces';

//TODO remove async from caches
class WorkspaceServicesCache extends Cache<WorkspaceServicesInfo> implements IWorkspaceServicesCache {
	private prefixKey = 'workspaceServices';

	cache(workspaceId: string, serviceId: string, data: WorkspaceServicesInfo): Promise<void> {
		const key = this.getCacheKey(workspaceId, serviceId);
		return Promise.resolve(this.set(key, data));
	}

	/** This should cache the same way fetchServices fetches them */
	cacheServices(workspaceId: string, services: Record<string, WorkspaceServicesInfo>): Promise<void> {
		for (const serviceId in services) {
			const key = this.getCacheKey(workspaceId, serviceId);
			this.set(key, services[serviceId]);
		}
		return Promise.resolve();
	}

	/** Returned result should mirror db.getWorkflowServices */
	fetchServices(workspaceId: string, services: Set<string>): Promise<Record<string, WorkspaceServicesInfo>> {
		const res = {};
		for (const serviceId of services) {
			const key = this.getCacheKey(workspaceId, serviceId);
			const service = this.get(key);
			if (!service) continue;
			// const { environments, name, type, meta } = service;
			// const { target, ssl } = environments[environmentId];
			res[serviceId] = service;
		}
		return Promise.resolve(res);
	}

	private getCacheKey(workspaceId: string, serviceId: string) {
		return `${this.prefixKey}:${workspaceId}:${serviceId}`;
	}
}

export default WorkspaceServicesCache;

// const cache: Map<string, Record<string, WorkspaceServicesInfo>> = {
// 	workspaceId1: {
// 		id1111: {
// 			id: 'id1111',
// 			name: 'Mongo',
// 			description: 'gRPC Mongo Service',
// 			tags: ['db', 'mongo', 'gRPC'],
// 			type: ServiceType.gRPC,
// 			proto: '',
// 			environments: {
//				prod_345: {
//					target: 'localhost:3444',
//					ssl: false,
//				}
//		}
// 		},
// 		id1112: {
// 			id: 'id1111',
// 			name: 'Mongo',
// 			description: 'gRPC Mongo Service',
// 			tags: ['db', 'mongo', 'gRPC'],
// 			type: ServiceType.gRPC,
// 			target: 'localhost:3005',
// 			ssl: false,
// 		},
// 	},
// };

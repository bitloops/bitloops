import { IBitloopsWorkflowDefinition } from '../../entities/workflow/definitions';
import { IWorkflowVersionMappingCache } from '../interfaces';
import Cache from './Cache';

class WorkflowVersionMappingCache extends Cache<string> implements IWorkflowVersionMappingCache {
    private prefixKey = 'workflowVersionMapping';

    constructor(max: number) {
        super(max);
    }

    cache(workflowId: string, workflowVersion: string) {
        console.log(`adding worfklow version mapping with id: ${workflowId} and version ${workflowVersion}`);
        const key = this.getCacheKey(workflowId);
        this.set(key, workflowVersion);
    }

    fetch(workflowId: string): Promise<string> {
        console.log(`fetching worfklow version mapping with id: ${workflowId}`);
        const key = this.getCacheKey(workflowId);
        const res = this.get(key);
        return Promise.resolve(res);
    }

    delete(workflowId: string) {
        console.log(`deleting worfklow version mapping with id: ${workflowId}`);
        const key = this.getCacheKey(workflowId);
        this.remove(key);
    }

    private getCacheKey(workflowId: string) {
        return `${this.prefixKey}:${workflowId}`;
    }
}

export default WorkflowVersionMappingCache;

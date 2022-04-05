import { WorkspaceSecretsInfo } from '../../entities/workflow/definitions';
import Cache from './Cache';
import { IWorkspaceSecretsCache } from '../interfaces';

class WorkspaceSecretsCache extends Cache<WorkspaceSecretsInfo> implements IWorkspaceSecretsCache {
	private prefixKey = 'workspaceSecrets';

	cache(workspaceId: string, secretId: string, data: WorkspaceSecretsInfo): Promise<void> {
		const key = this.getCacheKey(workspaceId, secretId);
		return Promise.resolve(this.set(key, data));
	}

	deleteSecret(workspaceId: string, secretId: string): Promise<void> {
		const key = this.getCacheKey(workspaceId, secretId);
		this.remove(key);
		return Promise.resolve();
	}

	cacheSecrets(workspaceId: string, secrets: Record<string, WorkspaceSecretsInfo>): Promise<void> {
		for (const secretId in secrets) {
			const key = this.getCacheKey(workspaceId, secretId);
			this.set(key, secrets[secretId]);
		}
		return Promise.resolve();
	}

	fetchSecrets(workspaceId: string, secretIds: string[]): Promise<Record<string, WorkspaceSecretsInfo>> {
		const res = {};
		for (let i = 0; i < secretIds.length; i++) {
			const secretId = secretIds[i];
			const key = this.getCacheKey(workspaceId, secretId);
			const secret = this.get(key);
			if (!secret) continue;
			res[secretId] = secret;
		}
		return Promise.resolve(res);
	}

	private getCacheKey(workspaceId: string, secretId: string) {
		return `${this.prefixKey}:${workspaceId}:${secretId}`;
	}
}

export default WorkspaceSecretsCache;

import { MQTopics } from '../../constants';
import { ILogger, IMQ } from '../interfaces';
import Options from '../Options';

class Logger implements ILogger {
	private mq: IMQ;
	private topic: string;

	constructor(mq: IMQ) {
		this.mq = mq;
		this.topic = `${Options.getVersion()}.${Options.getOption(MQTopics.ENGINE_LOGGER_TOPIC)}`;
	}

	async log(data: Record<string, unknown>): Promise<void> {
		this.mq.publish(this.topic, data);
	}
}

export default Logger;

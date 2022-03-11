import { AppOptions } from '../../constants';
import { ILogger, IMQ } from '../interfaces';
import Options from '../Options';

class Logger implements ILogger {
	private mq: IMQ;
	private topic: string;

	constructor(mq: IMQ) {
		this.mq = mq;
		this.topic = Options.getOption(AppOptions.REST_LOGGER_TOPIC) ? Options.getOption(AppOptions.REST_LOGGER_TOPIC) : 'test.rest.logger-topic';
	}

	async log(data: Record<string, unknown>): Promise<void> {
		this.mq.publish(this.topic, data);
	}
}

export default Logger;

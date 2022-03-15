import Bitloops from 'bitloops';

import bitloopsConfig from '../bitloops/bitloopsConfig';
import { DemoChat } from './DemoChat'; 

const bitloopsApp = Bitloops.initialize(bitloopsConfig);
const demoChat = new DemoChat.DemoChatClient(bitloopsApp);
const bitloops = {
  app: bitloopsApp,
  auth: bitloopsApp.auth,
  subscribe: bitloopsApp.subscribe.bind(bitloopsApp),
  demoChat,
};

export { DemoChat };
export default bitloops;

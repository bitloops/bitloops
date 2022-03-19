import Bitloops from 'bitloops';

import bitloopsConfig from '../bitloops/bitloopsConfig';
import { HelloWorld } from './HelloWorld'; 

const bitloopsApp = Bitloops.initialize(bitloopsConfig);
const helloWorld = new HelloWorld.HelloWorldClient(bitloopsApp);
const bitloops = {
  app: bitloopsApp,
  auth: bitloopsApp.auth,
  subscribe: bitloopsApp.subscribe.bind(bitloopsApp),
  helloWorld,
};

export { HelloWorld };
export default bitloops;

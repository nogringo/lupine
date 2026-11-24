import { ANNOUNCEMENTS, type Follower, type HubMessage } from "../lib/tabs";

// Tabs cannot hand each other a port: this shared worker passes followers' ports to the leader.

const scope = self as unknown as SharedWorkerGlobalScope;
const announcements = new BroadcastChannel(ANNOUNCEMENTS);
let leader: MessagePort | undefined;

scope.onconnect = (event) => {
  event.ports[0].onmessage = ({ data }: MessageEvent<HubMessage>) => {
    if (data.type === "lead") {
      leader = data.accept;
      announcements.postMessage(null);
    } else {
      // Lost when the leader is gone: the next one's announcement makes the tab ask again.
      leader?.postMessage({ tab: data.tab, port: data.port } satisfies Follower, [data.port]);
    }
  };
};

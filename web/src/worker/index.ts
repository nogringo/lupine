import * as Comlink from "comlink";
import { Service } from "./service";

Comlink.expose(new Service());

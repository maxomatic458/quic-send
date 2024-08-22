/* @refresh reload */
import { render } from "solid-js/web";
import { Router, Route } from "@solidjs/router";
import "./styles.css";

import App from "./Pages/App";
import Send from "./Pages/Send";
import Receive from "./Pages/Receive";

render(() => 
    <Router>
        <Route path="/" component={App} />
        <Route path="/send" component={Send} />
        <Route path="/receive" component={Receive} />
    </Router>, 
    document.getElementById("root") as HTMLElement
);

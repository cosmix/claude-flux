import { createBrowserRouter, type RouteObject } from "react-router";

import { RouteError } from "@/routes/error";
import { Ledger } from "@/routes/ledger";
import { Overview } from "@/routes/overview";
import { Shell } from "@/routes/shell";
import { StagePage } from "@/routes/stage";
import { TerminalPage } from "@/routes/terminal";

/// Route objects, exported so tests can build a memory router from them.
export const routes: RouteObject[] = [
  {
    path: "/",
    element: <Shell />,
    errorElement: <RouteError />,
    children: [
      { index: true, element: <Overview /> },
      { path: "ledger", element: <Ledger /> },
      { path: "stages/:stageId", element: <StagePage /> },
      { path: "terminal/:stageId", element: <TerminalPage /> },
    ],
  },
];

export const router = createBrowserRouter(routes);

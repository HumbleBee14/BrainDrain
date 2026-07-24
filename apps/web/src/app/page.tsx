import { auth } from "@clerk/nextjs/server";
import { redirect } from "next/navigation";

// The public marketing site (ekcron.com) is the front door now. The app's root
// just routes: signed-in users to the dashboard, everyone else to sign-in.
export default async function RootPage() {
  const { userId } = await auth();
  redirect(userId ? "/dashboard" : "/sign-in");
}

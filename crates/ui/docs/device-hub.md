The Device hub runs iOS simulators and Android emulators inside Zeron, so you and your agents can use the app you are building next to the chat. Devices can run on this machine or on a remote device host.

## Set it up

Open [Settings > Device hub](zeron://settings/device-hub) and switch on **Device hub**. Zeron installs and starts its device tools.

- **Platform support** shows whether iOS (Xcode and iOS Simulator) and Android (Android SDK and Emulator) are ready, and what to install if not. Either platform works without the other. Click **Refresh** after installing.
- **Check versions** shows the running, required, and installed device tools. When they are out of date, an **Update to** button appears.
- **Agent device access** lets new agent sessions start and control devices. It needs **Device hub** on, and turning **Device hub** off turns it off too.
- **Auto-show floating preview** (on by default) shows a floating preview when an agent opens a device. With it off, the device opens as a tab in the right pane.

Until the hub is set up, a device tab walks you through the same steps. You can leave agent control off and still use devices yourself.

## Open a device

In a chat, click **+** in the right pane and choose **Device**, or click **Device** in an empty pane. The tab lists your **iOS Simulators** and **Android Emulators**. Click **Start** for a stopped device or **Open** for a running one, or **Refresh devices** if one is missing.

**Device** is only available from a thread whose chat runs on this computer. Otherwise it is dimmed; hover it for the reason.

## Use the device

Click and drag on the live screen to touch it, and type to send keys. While an iOS simulator's screen has focus, Cmd+R goes to the simulator instead of toggling the right pane; on an Android emulator's screen it does nothing. The rail beside it has **Home**, **Rotate device**, **Save screenshot**, and **Device tools**, a drawer with settings such as **Appearance** and **Text size**, plus app launching and location. **Color filter** is there for iOS simulators only. **More device actions** has **Float device over chat** and **Close device panel**, plus **Power off device** for simulators and emulators (not physical devices).

![An iOS simulator in the right pane](device-hub-device.jpg)

To rename a device tab, double-click it, or right-click it and choose **Rename**.

A floating preview stays over the chat. Closing the right pane while a device tab is showing floats that device too. Use **Open in right panel** to dock it again, or **Close floating preview**.

## Remote device hosts

To use simulators or emulators on another machine, click **Add host** under **Device hosts**. Fill in **Name** and **SSH target**, optionally **Identity file** and **Port**, then click **Test connection** and **Save host**. Zeron connects over SSH and sets up the device tools there. That host's devices then appear in the device tab, grouped by host. Each saved host has its own **Test connection** and **Remove** buttons.

> With **Agent device access** on, you can ask an agent to open a simulator and test a flow itself.

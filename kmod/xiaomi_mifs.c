// SPDX-License-Identifier: GPL-2.0
/*
 * xiaomi_mifs.c - Native Linux WMI driver for Xiaomi Book Pro 14 (TM2424)
 *
 * Implements the Bitland MIFS WMI protocol (GUID B60BFB48-3E5B-49E4-A0E9-8CFFE1B3434B)
 * verified for TM2424 by Oksion/XiControl:
 *   - /dev/xiaomi_mifs (raw 32-byte MIFS transactions for XiControl GUI/CLI)
 *   - /sys/kernel/xiaomi_mifs/{perf_mode,charge_limit,adapter_watts,battery_health,mic_mute}
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/wmi.h>
#include <linux/mutex.h>
#include <linux/slab.h>
#include <linux/fs.h>
#include <linux/miscdevice.h>
#include <linux/uaccess.h>
#include <linux/kobject.h>
#include <linux/sysfs.h>
#include <linux/delay.h>

#define MIFS_METHOD_GUID "B60BFB48-3E5B-49E4-A0E9-8CFFE1B3434B"
#define MIFS_BUF_SIZE    32

#define MIFS_OP_GET      0xFA
#define MIFS_OP_SET      0xFB
#define MIFS_STATUS_OK   0x80

#define MIFS_CMD_PERF    0x08
#define MIFS_CMD_MIC     0x0A
#define MIFS_CMD_CHARGE  0x10

struct xiaomi_mifs_priv {
	struct wmi_device *wdev;
	struct mutex lock;
	u8 last_out[MIFS_BUF_SIZE];
};

static struct xiaomi_mifs_priv *g_mifs;
static struct kobject *mifs_kobj;

static int mifs_raw_call(struct xiaomi_mifs_priv *priv, const u8 in_pkt[MIFS_BUF_SIZE],
			 u8 out_pkt[MIFS_BUF_SIZE])
{
	struct wmi_buffer in_buf = {
		.length = MIFS_BUF_SIZE,
		.data = (void *)in_pkt,
	};
	struct wmi_buffer out_buf = { 0 };
	int ret;

	if (!priv || !priv->wdev)
		return -ENODEV;

	mutex_lock(&priv->lock);
	memset(out_pkt, 0, MIFS_BUF_SIZE);
	ret = wmidev_invoke_method(priv->wdev, 0, 1, &in_buf, &out_buf, 8);
	if (ret == 0 && out_buf.data) {
		size_t copy_len = min_t(size_t, out_buf.length, MIFS_BUF_SIZE);
		memcpy(out_pkt, out_buf.data, copy_len);
		memcpy(priv->last_out, out_pkt, MIFS_BUF_SIZE);
		kfree(out_buf.data);
	}
	mutex_unlock(&priv->lock);
	return ret;
}

static int mifs_cmd(u8 op, u8 cmd, u8 arg, u8 val, u8 out[MIFS_BUF_SIZE])
{
	u8 in[MIFS_BUF_SIZE] = { 0 };

	in[1] = op;
	in[3] = cmd;
	in[4] = arg;
	in[6] = val;
	return mifs_raw_call(g_mifs, in, out);
}

/* /dev/xiaomi_mifs file operations */
static ssize_t mifs_dev_write(struct file *file, const char __user *buf,
			      size_t count, loff_t *ppos)
{
	u8 in_pkt[MIFS_BUF_SIZE] = { 0 };
	u8 out_pkt[MIFS_BUF_SIZE] = { 0 };
	size_t len = min_t(size_t, count, MIFS_BUF_SIZE);
	int ret;

	if (!g_mifs)
		return -ENODEV;
	if (copy_from_user(in_pkt, buf, len))
		return -EFAULT;

	ret = mifs_raw_call(g_mifs, in_pkt, out_pkt);
	if (ret < 0)
		return ret;
	return count;
}

static ssize_t mifs_dev_read(struct file *file, char __user *buf,
			     size_t count, loff_t *ppos)
{
	u8 tmp[MIFS_BUF_SIZE];
	size_t len = min_t(size_t, count, MIFS_BUF_SIZE);

	if (!g_mifs)
		return -ENODEV;

	mutex_lock(&g_mifs->lock);
	memcpy(tmp, g_mifs->last_out, MIFS_BUF_SIZE);
	mutex_unlock(&g_mifs->lock);

	if (copy_to_user(buf, tmp, len))
		return -EFAULT;
	return len;
}

static const struct file_operations mifs_fops = {
	.owner = THIS_MODULE,
	.write = mifs_dev_write,
	.read  = mifs_dev_read,
	.llseek = noop_llseek,
};

static struct miscdevice mifs_miscdev = {
	.minor = MISC_DYNAMIC_MINOR,
	.name  = "xiaomi_mifs",
	.fops  = &mifs_fops,
	.mode  = 0666,
};

/* Sysfs attributes under /sys/kernel/xiaomi_mifs/ */
static ssize_t perf_mode_show(struct kobject *kobj, struct kobj_attribute *attr, char *buf)
{
	u8 out[MIFS_BUF_SIZE];
	const char *name = "unknown";

	if (mifs_cmd(MIFS_OP_GET, MIFS_CMD_PERF, 0, 0, out) < 0 || out[1] != MIFS_STATUS_OK)
		return -EIO;

	switch (out[4]) {
	case 0x02: name = "quiet"; break;
	case 0x03: name = "turbo"; break;
	case 0x04: name = "fullspeed"; break;
	case 0x09: name = "auto"; break;
	case 0x0A: name = "eco"; break;
	case 0x01: name = "balance"; break;
	}
	return sysfs_emit(buf, "%s (0x%02x)\n", name, out[4]);
}

static ssize_t perf_mode_store(struct kobject *kobj, struct kobj_attribute *attr,
			       const char *buf, size_t count)
{
	u8 out[MIFS_BUF_SIZE];
	u8 code = 0;

	if (sysfs_streq(buf, "quiet") || sysfs_streq(buf, "silent") || sysfs_streq(buf, "2"))
		code = 0x02;
	else if (sysfs_streq(buf, "turbo") || sysfs_streq(buf, "3"))
		code = 0x03;
	else if (sysfs_streq(buf, "fullspeed") || sysfs_streq(buf, "full") || sysfs_streq(buf, "4"))
		code = 0x04;
	else if (sysfs_streq(buf, "auto") || sysfs_streq(buf, "smart") || sysfs_streq(buf, "9"))
		code = 0x09;
	else if (sysfs_streq(buf, "eco") || sysfs_streq(buf, "10"))
		code = 0x0A;
	else
		return -EINVAL;

	if (mifs_cmd(MIFS_OP_SET, MIFS_CMD_PERF, code, 0, out) < 0 || out[1] != MIFS_STATUS_OK)
		return -EIO;
	return count;
}

static ssize_t charge_limit_show(struct kobject *kobj, struct kobj_attribute *attr, char *buf)
{
	u8 out[MIFS_BUF_SIZE];
	int pct = 100;

	if (mifs_cmd(MIFS_OP_GET, MIFS_CMD_CHARGE, 0x02, 0, out) < 0 || out[1] != MIFS_STATUS_OK)
		return -EIO;

	switch (out[6]) {
	case 0: pct = 100; break;
	case 1:
	case 4: pct = 80; break;
	case 5: pct = 70; break;
	case 6: pct = 60; break;
	case 7: pct = 50; break;
	case 8: pct = 40; break;
	default: pct = -1; break;
	}
	return sysfs_emit(buf, "%d\n", pct);
}

static ssize_t charge_limit_store(struct kobject *kobj, struct kobj_attribute *attr,
				  const char *buf, size_t count)
{
	u8 out[MIFS_BUF_SIZE];
	int pct;
	u8 code;

	if (kstrtoint(buf, 10, &pct) < 0)
		return -EINVAL;

	switch (pct) {
	case 100: code = 0; break;
	case 80:  code = 1; break;
	case 70:  code = 5; break;
	case 60:  code = 6; break;
	case 50:  code = 7; break;
	case 40:  code = 8; break;
	default:  return -EINVAL;
	}

	if (code != 0) {
		mifs_cmd(MIFS_OP_SET, MIFS_CMD_CHARGE, 0x02, 0, out);
		msleep(80);
	}
	if (mifs_cmd(MIFS_OP_SET, MIFS_CMD_CHARGE, 0x02, code, out) < 0 || out[1] != MIFS_STATUS_OK)
		return -EIO;
	return count;
}

static ssize_t adapter_watts_show(struct kobject *kobj, struct kobj_attribute *attr, char *buf)
{
	u8 out[MIFS_BUF_SIZE];

	if (mifs_cmd(MIFS_OP_GET, MIFS_CMD_CHARGE, 0x06, 0, out) < 0 || out[1] != MIFS_STATUS_OK)
		return -EIO;
	return sysfs_emit(buf, "%u\n", out[6]);
}

static ssize_t battery_health_show(struct kobject *kobj, struct kobj_attribute *attr, char *buf)
{
	u8 out[MIFS_BUF_SIZE];

	if (mifs_cmd(MIFS_OP_GET, MIFS_CMD_CHARGE, 0x01, 0, out) < 0 || out[1] != MIFS_STATUS_OK)
		return -EIO;
	return sysfs_emit(buf, "%u\n", out[6]);
}

static struct kobj_attribute perf_mode_attr = __ATTR_RW(perf_mode);
static struct kobj_attribute charge_limit_attr = __ATTR_RW(charge_limit);
static struct kobj_attribute adapter_watts_attr = __ATTR_RO(adapter_watts);
static struct kobj_attribute battery_health_attr = __ATTR_RO(battery_health);

static struct attribute *mifs_attrs[] = {
	&perf_mode_attr.attr,
	&charge_limit_attr.attr,
	&adapter_watts_attr.attr,
	&battery_health_attr.attr,
	NULL,
};

static const struct attribute_group mifs_attr_group = {
	.attrs = mifs_attrs,
};

static int xiaomi_mifs_probe(struct wmi_device *wdev, const void *context)
{
	struct xiaomi_mifs_priv *priv;
	int ret;

	priv = devm_kzalloc(&wdev->dev, sizeof(*priv), GFP_KERNEL);
	if (!priv)
		return -ENOMEM;

	priv->wdev = wdev;
	mutex_init(&priv->lock);
	dev_set_drvdata(&wdev->dev, priv);
	g_mifs = priv;

	ret = misc_register(&mifs_miscdev);
	if (ret)
		return ret;

	mifs_kobj = kobject_create_and_add("xiaomi_mifs", kernel_kobj);
	if (mifs_kobj) {
		if (sysfs_create_group(mifs_kobj, &mifs_attr_group))
			kobject_put(mifs_kobj);
	}

	dev_info(&wdev->dev, "Xiaomi MIFS WMI driver (TM2424 XiControl bridge) initialized\n");
	return 0;
}

static void xiaomi_mifs_remove(struct wmi_device *wdev)
{
	if (mifs_kobj) {
		sysfs_remove_group(mifs_kobj, &mifs_attr_group);
		kobject_put(mifs_kobj);
		mifs_kobj = NULL;
	}
	misc_deregister(&mifs_miscdev);
	g_mifs = NULL;
}

static const struct wmi_device_id xiaomi_mifs_id_table[] = {
	{ .guid_string = MIFS_METHOD_GUID },
	{ }
};
MODULE_DEVICE_TABLE(wmi, xiaomi_mifs_id_table);

static struct wmi_driver xiaomi_mifs_driver = {
	.driver = {
		.name = "xiaomi_mifs",
	},
	.id_table = xiaomi_mifs_id_table,
	.probe = xiaomi_mifs_probe,
	.remove = xiaomi_mifs_remove,
};

module_wmi_driver(xiaomi_mifs_driver);

MODULE_AUTHOR("XiControl Linux Port");
MODULE_DESCRIPTION("Xiaomi Book Pro 14 (TM2424) MIFS WMI Driver for XiControl");
MODULE_LICENSE("GPL");

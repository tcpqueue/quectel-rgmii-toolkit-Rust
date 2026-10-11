(function (global) {
  'use strict';
  const root = global.SimpleAdmin;
  const words = {
    "通知内容": "What to send",
    "渠道在下方统一配置；开启的渠道可分别用于短信转发和断网恢复通知。推送失败会在 3 分钟内重试，渠道和开关的修改都需要点“保存”。": "Channels are set up below; each enabled channel can be used for SMS forwarding, outage notices or both. Failed deliveries are retried for 3 minutes. Changes to channels and switches take effect after Save.",
    "短信转发": "SMS forwarding",
    "转发所有新短信": "Forward every new SMS",
    "断网恢复通知": "Outage notice",
    "网络恢复后发送一次，包含断网时间、恢复时间和时长": "Sent once the connection is back, with the outage start, end and length",
    "断网至少多久才通知（秒）": "Notify only after outages of at least (seconds)",
    "使用渠道": "Channels",
    "未配置": "Not set up",
    "请先在下方配置并启用渠道": "Set up and enable a channel below first",
    "需要开启总览的延迟监测才能判断断网": "Needs the latency probe on the overview page to detect outages",
    "通知渠道": "Channels",
    "设备名称": "Device name",
    "重试上限": "Retry window",
    "分钟": "min",
    "等待推送": "Queued",
    "运行中": "Running",
    "等待首次读取": "Initializing",
    "已停用": "Disabled",
    "已启用": "Enabled",
    "推送平台": "Channel",
    "签名密钥": "Signing secret",
    "可选": "Optional",
    "测试推送": "Test notification",
    "推送中…": "Sending…",
    "清除凭据": "Clear credentials",
    "待清除": "Removal pending",
    "近期推送": "Recent notifications",
    "最近 50 条": "Last 50 results",
    "时间": "Time",
    "类型": "Type",
    "发送号码": "Sender",
    "结果": "Result",
    "暂无推送记录": "No notifications yet",
    "测试成功": "Test delivered",
    "短信": "SMS",
    "断网恢复": "Outage",
    "测试": "Test",
    "sent": "Delivered",
    "retrying": "Retrying",
    "failed": "Failed",
    "test": "Test notification",
    "SMS read failed": "SMS read failed",
    "retry window expired": "Retry window expired",
    "queue capacity exceeded": "Queue is full",
    "network or TLS request failed": "Connection failed",
    "enable at least one channel": "Choose at least one enabled channel for SMS forwarding",
    "choose an enabled channel for outage notices": "Choose at least one enabled channel for outage notices",
    "invalid outage threshold": "Outage threshold must be 10–3600 seconds",
    "invalid ServerChan Turbo SendKey": "Invalid ServerChan Turbo SendKey",
    "invalid webhook URL": "Invalid Webhook URL",
    "webhook does not match selected platform": "Webhook does not match the channel",
    "webhook token missing": "Webhook token missing",
    "invalid device name": "Invalid device name",
    "wait before testing again": "Wait before testing again"
  };
  const chinese = {sent:'已送达',retrying:'重试中',failed:'失败',test:'测试推送','SMS read failed':'短信读取失败','retry window expired':'已超过 3 分钟重试上限','queue capacity exceeded':'推送队列已满','network or TLS request failed':'网络连接或证书验证失败','enable at least one channel':'短信转发请至少选择一个已启用的渠道','choose an enabled channel for outage notices':'断网恢复通知请至少选择一个已启用的渠道','invalid outage threshold':'断网时长阈值需在 10–3600 秒之间','invalid ServerChan Turbo SendKey':'Server酱 Turbo SendKey 无效','invalid webhook URL':'Webhook URL 无效','webhook does not match selected platform':'Webhook 地址与平台不匹配','webhook token missing':'Webhook 地址缺少令牌','invalid device name':'设备名称无效','wait before testing again':'请稍后再次测试'};
  root.Pages.forwarding = function () {
    return {
      language: root.Lang.getCurrentLanguage(), enabled:false,sms_enabled:true,delete_after_day:false,device_name:'',channels:[],selected:'serverchan',
      sms_platforms:[],outage_enabled:false,outage_platforms:[],outage_min_seconds:60,probe_enabled:true,
      loaded:false,busy:false,dirty:false,testing:'',message:'',failed:false,error:'',queued:0,ready:false,records:[],loading:false,timer:null,
      t(key) {
        if (this.language==='zh-CN') return chinese[key] || key;
        return words[key] || root.Lang.t(key);
      },
      webhookExample: '{"event":"sms.received","id":"...","device":"RM520N-EU","sender":"10086","received_at":"...","text":"...","part":1,"parts":1}\n{"event":"network.restored","id":"...","device":"RM520N-EU","lost_at":"...","restored_at":"...","seconds":185,"text":"..."}',
      guide(key) { return root.ForwardingGuides.get(key,this.language); },
      platform(key) {if(key==='sim') return this.t('本卡短信');return ({serverchan:'Server酱 Turbo',wecom:'企业微信',dingtalk:'钉钉',feishu:'飞书',webhook:'Webhook'})[key] && (this.language==='zh-CN' ? ({serverchan:'Server酱 Turbo',wecom:'企业微信',dingtalk:'钉钉',feishu:'飞书',webhook:'Webhook'})[key] : ({serverchan:'ServerChan Turbo',wecom:'WeCom',dingtalk:'DingTalk',feishu:'Feishu',webhook:'Webhook'})[key]) || key;},
      eventName(key) {return this.t(({sms:'短信',outage:'断网恢复',test:'测试'})[key] || '短信');},
      time(value) {return new Intl.DateTimeFormat(this.language,{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',second:'2-digit'}).format(new Date(value));},
      changed() {for(const c of this.channels) {if(c.clear && (c.url || c.token || c.secret || c.number)) c.clear=false;}this.dirty=true;this.message='';},
      // A channel counts as set up once it is switched on and has its credentials.
      ready_channel(c) {return c.enabled && (c.platform==='sim' ? !!(c.number||'').trim() : (c.has_url||c.has_token||!!(c.url||c.token||'').trim()));},
      usable() {return this.channels.filter(c=>this.ready_channel(c));},
      togglePlatform(list,platform) {const i=this[list].indexOf(platform);if(i===-1)this[list].push(platform);else this[list].splice(i,1);this.changed();},
      async request(path,data) {
        const response=await root.Api.request(path,data ? {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(data)} : {});
        const result=await response.json();
        if (!response.ok || result.ok===false) throw new Error(result.error || 'Request failed');
        return result;
      },
      apply(data,form) {
        this.queued=data.queued;this.ready=data.ready;this.records=data.records;this.error=data.error;
        this.sms_enabled=data.sms_enabled;this.delete_after_day=data.delete_after_day;
        if (form) {
          this.enabled=data.enabled;this.device_name=data.device_name;this.channels=data.channels.map(c=>({...c,url:'',token:'',secret:'',clear:false}));
          this.sms_platforms=[...(data.sms_platforms||[])];this.outage_enabled=!!data.outage_enabled;this.outage_platforms=[...(data.outage_platforms||[])];this.outage_min_seconds=data.outage_min_seconds||60;
          this.dirty=false;this.loaded=true;
        }
      },
      async refresh() {
        if (this.loading || this.busy) return;
        this.loading=true;
        try {this.apply(await this.request('/api/forwarding'),!this.loaded);} catch (error) {this.message=error.message;this.failed=true;} finally {this.loading=false;}
        try {const watchdog=await root.Api.networkData({action:'watchdog'});this.probe_enabled=watchdog.probe_enabled!==false;} catch (_) {}
      },
      async save() {
        this.busy=true;this.message='';
        try {
          const channels=this.channels.map(({platform,enabled,url,token,secret,number,clear})=>({platform,enabled,url:url.trim(),token:token.trim(),secret:secret.trim(),number:(number||'').trim(),clear}));
          const current=await this.request('/api/forwarding');this.sms_enabled=current.sms_enabled;this.delete_after_day=current.delete_after_day;
          const minimum=Math.round(Number(this.outage_min_seconds));
          this.apply(await this.request('/api/forwarding/save',{enabled:this.enabled,sms_enabled:this.sms_enabled,delete_after_day:this.delete_after_day,device_name:this.device_name,channels,
            sms_platforms:this.sms_platforms,outage_enabled:this.outage_enabled,outage_platforms:this.outage_platforms,outage_min_seconds:Number.isFinite(minimum)?minimum:0,
            utc_offset_minutes:-new Date().getTimezoneOffset(),language:this.language==='en'?'en':'zh-CN'}),true);
          this.message='已保存';this.failed=false;
        } catch (error) {this.message=error.message;this.failed=true;} finally {this.busy=false;}
      },
      async test(platform) {
        if (this.dirty || this.testing) return;
        this.testing=platform;this.message='';
        try {await this.request('/api/forwarding/test',{platform});this.message='测试成功';this.failed=false;} catch (error) {this.message=error.message;this.failed=true;} finally {this.testing='';await this.refresh();}
      },
      clear(c) {c.clear=true;c.enabled=false;c.url='';c.token='';c.secret='';c.number='';c.has_url=false;c.has_token=false;c.has_secret=false;this.changed();},
      init() {
        this.refresh();
        const start=()=>{if (!this.timer) this.timer=setInterval(()=>this.refresh(),5000);};
        const sync=()=>{const page=document.querySelector('[data-page="notify"]');if(page && page.classList.contains('active')){this.refresh();start();}else{clearInterval(this.timer);this.timer=null;}};
        global.addEventListener('simpleadmin:page-changed',sync);
        global.addEventListener('simpleadmin:sms-settings',()=>this.refresh());
        global.addEventListener('simpleadmin:language-changed',()=>{this.language=root.Lang.getCurrentLanguage();});
        sync();
      }
    };
  };
})(window);
